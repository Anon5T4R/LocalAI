// Comparador de modelos via llama-bench (processo filho), no padrao do
// quant.rs: resolve a pasta binaries, spawn com CREATE_NO_WINDOW + prioridade
// baixa, kill no cancelamento/saida do app e erros em portugues.
//
// Os modelos rodam SEQUENCIALMENTE (um processo por vez): dois llama-bench
// simultaneos disputam CPU/RAM e os numeros dos dois saem errados.
//
// Saida: `-o json` (confirmado no b9723 empacotado: o JSON sai LIMPO no
// stdout; os logs de load/progresso vao para o stderr). Cada modelo gera uma
// lista com um objeto por teste: pp (n_prompt=512, n_gen=0) e tg (n_prompt=0,
// n_gen=128), com o tok/s medio em `avg_ts`.
//
// Eventos: "bench-progress" {index, total, path} ao iniciar cada modelo,
// "bench-result" {path, ppTps, tgTps, error?} ao terminar cada um e
// "bench-done" {cancelled} no fim da fila (ou no cancelamento).

use serde::Serialize;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
// Prioridade baixa: o bench satura a CPU por minutos e nao pode travar a maquina.
#[cfg(windows)]
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

#[cfg(windows)]
const BENCH_BIN: &str = "llama-bench.exe";
#[cfg(not(windows))]
const BENCH_BIN: &str = "llama-bench";

static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
pub struct BenchState {
    pub child: Mutex<Option<Child>>,
}

#[derive(Clone, Serialize)]
struct BenchProgress {
    index: u32,
    total: u32,
    path: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchResult {
    path: String,
    pp_tps: Option<f64>,
    tg_tps: Option<f64>,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
struct BenchDone {
    cancelled: bool,
}

/// Extrai (pp tok/s, tg tok/s) do JSON do `llama-bench -o json`.
/// Puro e testavel. O teste de prompt tem n_prompt>0 e n_gen=0; o de geracao,
/// n_prompt=0 e n_gen>0; `avg_ts` e a media de tok/s das repeticoes.
fn parse_bench_json(out: &str) -> Result<(Option<f64>, Option<f64>), String> {
    let v: serde_json::Value = serde_json::from_str(out.trim())
        .map_err(|e| format!("Saida do llama-bench nao e JSON valido: {e}"))?;
    let arr = v
        .as_array()
        .ok_or_else(|| "Saida do llama-bench nao e uma lista JSON.".to_string())?;
    let mut pp: Option<f64> = None;
    let mut tg: Option<f64> = None;
    for item in arr {
        let n_prompt = item.get("n_prompt").and_then(|x| x.as_u64()).unwrap_or(0);
        let n_gen = item.get("n_gen").and_then(|x| x.as_u64()).unwrap_or(0);
        let ts = item.get("avg_ts").and_then(|x| x.as_f64());
        if n_prompt > 0 && n_gen == 0 {
            pp = ts.or(pp);
        } else if n_gen > 0 && n_prompt == 0 {
            tg = ts.or(tg);
        }
    }
    if pp.is_none() && tg.is_none() {
        return Err("Saida do llama-bench sem medidas de prompt/geracao.".into());
    }
    Ok((pp, tg))
}

/// Roda o llama-bench para UM modelo e devolve (pp tok/s, tg tok/s).
/// Bloqueia a thread chamadora (que e a thread da fila, nao a main).
fn run_one(
    app: &AppHandle,
    exe: &Path,
    bin_dir: &Path,
    model: &str,
) -> Result<(Option<f64>, Option<f64>), String> {
    // Flags enxutas: defaults do bench (pp512/tg128), 3 repeticoes em vez de 5
    // pra nao demorar demais, JSON no stdout e prioridade baixa tambem nas
    // threads internas (--prio -1), alem da classe de processo do Windows.
    let mut cmd = Command::new(exe);
    cmd.arg("-m")
        .arg(model)
        .arg("-o")
        .arg("json")
        .arg("-r")
        .arg("3")
        .arg("--prio")
        .arg("-1")
        .current_dir(bin_dir) // DLLs ggml-*/llama.dll
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Falha ao iniciar o llama-bench: {e}"))?;

    let mut stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Guarda o Child para o cancel/kill_on_exit alcancarem o processo.
    let state = app.state::<BenchState>();
    *state.child.lock().unwrap() = Some(child);

    // stderr: so a cauda, para compor uma mensagem de erro legivel.
    let err_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let tail2 = err_tail.clone();
    let jerr = stderr.map(|s| {
        std::thread::spawn(move || {
            let reader = BufReader::new(s);
            for line in reader.lines().map_while(Result::ok) {
                let mut v = tail2.lock().unwrap();
                v.push(line);
                if v.len() > 15 {
                    v.remove(0);
                }
            }
        })
    });

    // stdout inteiro = o JSON (poucos KB; ler tudo evita deadlock de pipe).
    let mut out = String::new();
    if let Some(s) = stdout.as_mut() {
        let _ = s.read_to_string(&mut out);
    }
    if let Some(j) = jerr {
        let _ = j.join();
    }
    let status = {
        let state = app.state::<BenchState>();
        let child = state.child.lock().unwrap().take();
        child.map(|mut c| c.wait())
    };

    if CANCELLED.load(Ordering::SeqCst) {
        return Err("Comparacao cancelada.".into());
    }
    let ok = matches!(status, Some(Ok(s)) if s.success());
    if !ok {
        let tail = err_tail.lock().unwrap().join("\n");
        return Err(format!(
            "O llama-bench terminou com erro. Ultimas linhas do log:\n{tail}"
        ));
    }
    parse_bench_json(&out)
}

/// Inicia a fila de benchmarks em background e retorna na hora.
/// Progresso em "bench-progress"/"bench-result"; fim em "bench-done".
pub fn start(app: &AppHandle, paths: Vec<String>) -> Result<(), String> {
    if paths.is_empty() {
        return Err("Selecione ao menos um modelo para comparar.".into());
    }
    for p in &paths {
        if !PathBuf::from(p).is_file() {
            return Err(format!("Arquivo nao encontrado: {p}"));
        }
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Ja existe uma comparacao em andamento.".into());
    }
    CANCELLED.store(false, Ordering::SeqCst);

    // A partir daqui, qualquer erro precisa devolver o RUNNING.
    let prep = || -> Result<(PathBuf, PathBuf), String> {
        let bin_dir = crate::server::resolve_binaries_dir(app)
            .ok_or_else(|| "Nao encontrei a pasta binaries do llama.cpp.".to_string())?;
        let exe = bin_dir.join(BENCH_BIN);
        if !exe.is_file() {
            return Err(format!("Nao encontrei {BENCH_BIN} em {}.", bin_dir.display()));
        }
        Ok((exe, bin_dir))
    };
    let (exe, bin_dir) = match prep() {
        Ok(v) => v,
        Err(e) => {
            RUNNING.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };

    let app = app.clone();
    std::thread::spawn(move || {
        let total = paths.len() as u32;
        for (i, path) in paths.iter().enumerate() {
            if CANCELLED.load(Ordering::SeqCst) {
                break;
            }
            let _ = app.emit(
                "bench-progress",
                BenchProgress {
                    index: i as u32 + 1,
                    total,
                    path: path.clone(),
                },
            );
            match run_one(&app, &exe, &bin_dir, path) {
                Ok((pp, tg)) => {
                    let _ = app.emit(
                        "bench-result",
                        BenchResult {
                            path: path.clone(),
                            pp_tps: pp,
                            tg_tps: tg,
                            error: None,
                        },
                    );
                }
                Err(e) => {
                    if CANCELLED.load(Ordering::SeqCst) {
                        break; // cancelou no meio: nao emite resultado-erro
                    }
                    // erro num modelo nao derruba a fila: registra e segue
                    let _ = app.emit(
                        "bench-result",
                        BenchResult {
                            path: path.clone(),
                            pp_tps: None,
                            tg_tps: None,
                            error: Some(e),
                        },
                    );
                }
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
        let _ = app.emit(
            "bench-done",
            BenchDone {
                cancelled: CANCELLED.load(Ordering::SeqCst),
            },
        );
    });

    Ok(())
}

/// Cancela a fila (mata o processo atual; a thread da fila para e emite o
/// "bench-done" com cancelled=true).
pub fn cancel(app: &AppHandle) {
    CANCELLED.store(true, Ordering::SeqCst);
    let state = app.state::<BenchState>();
    let guard = state.child.lock();
    if let Ok(mut guard) = guard {
        if let Some(child) = guard.as_mut() {
            let _ = child.kill();
        }
    }
}

/// Encerra o bench no fim do app (RunEvent::Exit) — mesmo papel do
/// quant::kill_on_exit: processo filho nunca sobrevive a janela.
pub fn kill_on_exit(app: &AppHandle) {
    CANCELLED.store(true, Ordering::SeqCst);
    let state = app.state::<BenchState>();
    let mut guard = match state.child.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Amostra REAL (enxugada) do `llama-bench -o json` do b9723 empacotado,
    // rodado no stories15M: um objeto pp (n_prompt=512, n_gen=0) e um tg
    // (n_prompt=0, n_gen=128); tok/s medio em avg_ts.
    const SAMPLE: &str = r#"[
  {
    "build_commit": "b14e3fb90",
    "build_number": 9723,
    "model_filename": "stories15M.gguf",
    "model_type": "llama ?B Q4_0",
    "model_size": 18350208,
    "n_prompt": 512,
    "n_gen": 0,
    "n_depth": 0,
    "avg_ns": 30582650,
    "avg_ts": 16827.402046,
    "stddev_ts": 1700.116102,
    "samples_ts": [ 15625.2, 18029.6 ]
  },
  {
    "build_commit": "b14e3fb90",
    "build_number": 9723,
    "model_filename": "stories15M.gguf",
    "model_type": "llama ?B Q4_0",
    "model_size": 18350208,
    "n_prompt": 0,
    "n_gen": 128,
    "n_depth": 0,
    "avg_ns": 258108300,
    "avg_ts": 496.903264,
    "stddev_ts": 31.325494,
    "samples_ts": [ 474.753, 519.054 ]
  }
]"#;

    #[test]
    fn json_real_do_bench() {
        let (pp, tg) = parse_bench_json(SAMPLE).unwrap();
        assert!((pp.unwrap() - 16827.402046).abs() < 1e-6);
        assert!((tg.unwrap() - 496.903264).abs() < 1e-6);
    }

    #[test]
    fn json_so_com_um_teste() {
        // se um dos testes faltar, o outro ainda vale
        let only_tg = r#"[{ "n_prompt": 0, "n_gen": 128, "avg_ts": 12.5 }]"#;
        let (pp, tg) = parse_bench_json(only_tg).unwrap();
        assert_eq!(pp, None);
        assert_eq!(tg, Some(12.5));
    }

    #[test]
    fn entradas_invalidas_dao_erro_legivel() {
        // tabela markdown (formato default sem -o json) NAO e JSON
        assert!(parse_bench_json("| model | size | pp512 t/s |").is_err());
        // JSON valido mas sem medidas
        assert!(parse_bench_json("[]").is_err());
        assert!(parse_bench_json(r#"[{"n_prompt":0,"n_gen":0}]"#).is_err());
        // objeto em vez de lista
        assert!(parse_bench_json(r#"{"a":1}"#).is_err());
        assert!(parse_bench_json("").is_err());
    }

    #[test]
    fn teste_misto_pg_nao_conta() {
        // -pg gera testes com n_prompt>0 E n_gen>0: nao sao pp puro nem tg puro
        let mixed = r#"[
          { "n_prompt": 512, "n_gen": 128, "avg_ts": 999.0 },
          { "n_prompt": 512, "n_gen": 0, "avg_ts": 100.0 },
          { "n_prompt": 0, "n_gen": 128, "avg_ts": 10.0 }
        ]"#;
        let (pp, tg) = parse_bench_json(mixed).unwrap();
        assert_eq!(pp, Some(100.0));
        assert_eq!(tg, Some(10.0));
    }
}
