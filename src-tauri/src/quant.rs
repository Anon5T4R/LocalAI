// Quantiza um GGUF chamando o llama-quantize empacotado (processo filho),
// no padrao do server.rs: resolve a pasta binaries, spawn com CREATE_NO_WINDOW,
// pump de stdout/stderr por linha e kill no cancelamento/saida do app.
//
// Progresso: o llama-quantize imprime uma linha "[   i/   n] nome_do_tensor..."
// por tensor. IMPORTANTE (medido no b9723/b10066): essas linhas saem no STDERR
// (o log unificado do llama.cpp), nao no stdout — por isso os DOIS streams sao
// parseados. Cada match vira um evento "quant-progress" {done, total}; o fim
// (sucesso, erro ou cancelamento) vira "quant-done" {path, error}.
//
// A saida e escrita como "<final>.part" e renomeada no sucesso — cancelar ou
// falhar nunca deixa um .gguf pela metade na pasta de modelos.

use serde::Serialize;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
// Prioridade baixa: quantizar e CPU-bound e nao pode matar a maquina do usuario.
#[cfg(windows)]
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

#[cfg(windows)]
const QUANTIZE_BIN: &str = "llama-quantize.exe";
#[cfg(not(windows))]
const QUANTIZE_BIN: &str = "llama-quantize";

/// Alvos oferecidos na v1. O backend valida contra esta lista para nunca
/// passar um argumento arbitrario do front ao processo.
const ALLOWED_TYPES: [&str; 5] = ["Q4_K_M", "Q5_K_M", "Q6_K", "Q8_0", "Q4_0"];

static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
pub struct QuantState {
    pub child: Mutex<Option<Child>>,
}

#[derive(Clone, Serialize)]
struct QuantProgress {
    done: u32,
    total: u32,
}

#[derive(Clone, Serialize)]
struct QuantDone {
    path: Option<String>,
    error: Option<String>,
}

/// Extrai "(done, total)" de uma linha "[   i/   n] tensor...".
/// Puro e testavel; devolve None para qualquer linha que nao seja de progresso.
fn parse_progress(line: &str) -> Option<(u32, u32)> {
    let rest = line.trim_start().strip_prefix('[')?;
    let (inside, _) = rest.split_once(']')?;
    let (a, b) = inside.split_once('/')?;
    let done: u32 = a.trim().parse().ok()?;
    let total: u32 = b.trim().parse().ok()?;
    if total == 0 || done > total {
        return None;
    }
    Some((done, total))
}

/// Caminho de saida ao lado do input: "nome-<TIPO>.gguf".
/// Nunca sobrescreve: se ja existe, tenta "nome-<TIPO>-2.gguf", "-3", ...
fn output_path(input: &Path, out_type: &str) -> Result<PathBuf, String> {
    let dir = input
        .parent()
        .ok_or_else(|| "Arquivo de origem sem pasta pai.".to_string())?;
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| "Nome do arquivo de origem invalido.".to_string())?;
    let first = dir.join(format!("{stem}-{out_type}.gguf"));
    if !first.exists() {
        return Ok(first);
    }
    for i in 2..1000 {
        let cand = dir.join(format!("{stem}-{out_type}-{i}.gguf"));
        if !cand.exists() {
            return Ok(cand);
        }
    }
    Err("Ja existem saidas demais com esse nome na pasta.".into())
}

fn finish(app: &AppHandle, path: Option<String>, error: Option<String>) {
    RUNNING.store(false, Ordering::SeqCst);
    let _ = app.emit("quant-done", QuantDone { path, error });
}

/// Inicia a quantizacao em background e retorna na hora com o caminho final
/// planejado. Progresso em "quant-progress"; conclusao em "quant-done".
pub fn start(app: &AppHandle, input: String, out_type: String) -> Result<String, String> {
    if !ALLOWED_TYPES.contains(&out_type.as_str()) {
        return Err(format!("Tipo de quantizacao nao suportado: {out_type}"));
    }
    let input_path = PathBuf::from(&input);
    if !input_path.is_file() {
        return Err(format!("Arquivo de origem nao encontrado: {input}"));
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Ja existe uma quantizacao em andamento.".into());
    }
    CANCELLED.store(false, Ordering::SeqCst);

    // A partir daqui, qualquer erro precisa devolver o RUNNING.
    let inner = || -> Result<String, String> {
        let bin_dir = crate::server::resolve_binaries_dir(app)
            .ok_or_else(|| "Nao encontrei a pasta binaries do llama.cpp.".to_string())?;
        let exe = bin_dir.join(QUANTIZE_BIN);
        if !exe.is_file() {
            return Err(format!(
                "Nao encontrei {QUANTIZE_BIN} em {}.",
                bin_dir.display()
            ));
        }

        let final_path = output_path(&input_path, &out_type)?;
        // .part: cancelamento/erro apaga; so o sucesso renomeia para .gguf
        // (tambem evita que o scanner de modelos liste um arquivo pela metade).
        let part_path = PathBuf::from(format!("{}.part", final_path.display()));

        // --allow-requantize: a UI oferece Q8_0 como origem valida (re-quantizar
        // de Q8_0 e aceitavel); para F16/F32/BF16 a flag e inofensiva (nao ha
        // tensor ja quantizado para "re" quantizar).
        let mut cmd = Command::new(&exe);
        cmd.arg("--allow-requantize")
            .arg(&input_path)
            .arg(&part_path)
            .arg(&out_type)
            .current_dir(&bin_dir) // DLLs ggml-*/llama.dll
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Falha ao iniciar o llama-quantize: {e}"))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let state = app.state::<QuantState>();
        *state.child.lock().unwrap() = Some(child);

        // Cauda do stderr para compor uma mensagem de erro legivel se falhar.
        let err_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        // Pump: parseia progresso nos DOIS streams (o quantize loga no stderr).
        let pump = |app: AppHandle,
                    stream: Box<dyn std::io::Read + Send>,
                    tail: Option<Arc<Mutex<Vec<String>>>>| {
            std::thread::spawn(move || {
                let reader = BufReader::new(stream);
                for line in reader.lines().map_while(Result::ok) {
                    if let Some((done, total)) = parse_progress(&line) {
                        let _ = app.emit("quant-progress", QuantProgress { done, total });
                    }
                    if let Some(t) = &tail {
                        let mut v = t.lock().unwrap();
                        v.push(line);
                        if v.len() > 15 {
                            v.remove(0);
                        }
                    }
                }
            })
        };
        let mut joins = Vec::new();
        if let Some(out) = stdout {
            joins.push(pump(app.clone(), Box::new(out), None));
        }
        if let Some(err) = stderr {
            joins.push(pump(app.clone(), Box::new(err), Some(err_tail.clone())));
        }

        // Thread de espera: junta os pumps, colhe o exit code e finaliza.
        let app2 = app.clone();
        let final2 = final_path.clone();
        std::thread::spawn(move || {
            for j in joins {
                let _ = j.join();
            }
            let status = {
                let state = app2.state::<QuantState>();
                let child = state.child.lock().unwrap().take();
                child.map(|mut c| c.wait())
            };
            let part = PathBuf::from(format!("{}.part", final2.display()));

            if CANCELLED.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&part);
                finish(&app2, None, Some("Quantizacao cancelada.".into()));
                return;
            }
            let ok = matches!(status, Some(Ok(s)) if s.success());
            if !ok {
                let _ = std::fs::remove_file(&part);
                let tail = err_tail.lock().unwrap().join("\n");
                finish(
                    &app2,
                    None,
                    Some(format!(
                        "O llama-quantize terminou com erro. Ultimas linhas do log:\n{tail}"
                    )),
                );
                return;
            }
            if let Err(e) = std::fs::rename(&part, &final2) {
                let _ = std::fs::remove_file(&part);
                finish(&app2, None, Some(format!("Nao renomeei o .part: {e}")));
                return;
            }
            finish(&app2, Some(final2.to_string_lossy().into_owned()), None);
        });

        Ok(final_path.to_string_lossy().into_owned())
    };

    match inner() {
        Ok(p) => Ok(p),
        Err(e) => {
            RUNNING.store(false, Ordering::SeqCst);
            Err(e)
        }
    }
}

/// Cancela a quantizacao em andamento (mata o processo; a thread de espera
/// apaga o .part e emite o "quant-done" com o aviso).
pub fn cancel(app: &AppHandle) {
    CANCELLED.store(true, Ordering::SeqCst);
    let state = app.state::<QuantState>();
    let guard = state.child.lock();
    if let Ok(mut guard) = guard {
        if let Some(child) = guard.as_mut() {
            let _ = child.kill();
        }
    }
}

/// Encerra o quantize no fim do app (RunEvent::Exit) — mesmo papel do
/// server::kill_on_exit: processo filho nunca sobrevive a janela.
pub fn kill_on_exit(app: &AppHandle) {
    let state = app.state::<QuantState>();
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

    #[test]
    fn progresso_formato_real_do_quantize() {
        // linha real do b9723/b10066 (sai no stderr)
        let l = "[   1/  57] output.weight                        - [   288,  32000,      1,      1], type =   q4_0, converting to q8_0 .. size =     4.39 MiB";
        assert_eq!(parse_progress(l), Some((1, 57)));
        let l2 = "[  57/  57] blk.5.ffn_up.weight                  - [   288,    768,      1,      1], type =   q4_0, converting to q8_0 .. size =     0.12 MiB ->     0.22 MiB";
        assert_eq!(parse_progress(l2), Some((57, 57)));
        // sem padding tambem vale
        assert_eq!(parse_progress("[291/291] x"), Some((291, 291)));
    }

    #[test]
    fn progresso_ignora_linhas_de_log() {
        assert_eq!(parse_progress("llama_model_loader: - type  f32: 48 tensors"), None);
        assert_eq!(parse_progress("load_backend: loaded RPC backend"), None);
        assert_eq!(parse_progress("[ERROR] algo"), None);
        assert_eq!(parse_progress("[1/0] x"), None); // total zero
        assert_eq!(parse_progress("[9/5] x"), None); // done > total
        assert_eq!(parse_progress(""), None);
        // dimensoes de tensor entre colchetes NAO podem virar progresso:
        // "[   288,  32000, ...]" nao tem '/' antes do ']'
        assert_eq!(parse_progress("[   288,  32000,      1,      1]"), None);
    }

    #[test]
    fn saida_ao_lado_sem_sobrescrever() {
        let dir = std::env::temp_dir().join(format!("quant-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("modelo-f16.gguf");
        std::fs::write(&input, b"x").unwrap();

        let p1 = output_path(&input, "Q4_K_M").unwrap();
        assert_eq!(p1, dir.join("modelo-f16-Q4_K_M.gguf"));

        // simulou existir: proxima chamada sufixa -2, depois -3
        std::fs::write(&p1, b"x").unwrap();
        let p2 = output_path(&input, "Q4_K_M").unwrap();
        assert_eq!(p2, dir.join("modelo-f16-Q4_K_M-2.gguf"));
        std::fs::write(&p2, b"x").unwrap();
        let p3 = output_path(&input, "Q4_K_M").unwrap();
        assert_eq!(p3, dir.join("modelo-f16-Q4_K_M-3.gguf"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
