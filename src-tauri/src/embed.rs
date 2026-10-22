// Segundo llama-server, SÓ para embeddings (--embedding): converte texto em
// vetores para a busca semântica. Sobe lazy (1ª indexação/busca), morre com
// o app, e usa QUALQUER GGUF de embedding que o usuário apontar — o app não
// fixa modelo (o llama.cpp valida na prática).
//
// Por que um segundo processo: modelos de chat não servem como embedding
// (qualidade ruim) e o llama-server só expõe /v1/embeddings com --embedding.
// Modelos de embedding são pequenos (~100-600 MB) e rodam bem na CPU, sem
// disputar a GPU/RAM do modelo de chat.

use crate::server::resolve_binaries_dir;
use serde::Serialize;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
const LLAMA_SERVER_BIN: &str = "llama-server.exe";
#[cfg(not(windows))]
const LLAMA_SERVER_BIN: &str = "llama-server";

/// Porta preferida (faixa de IA da suíte: 8104 Agenda, 8105 Scribe).
const EMBED_PORT: u16 = 8106;

#[derive(Clone, Serialize)]
pub struct EmbedInfo {
    pub port: u16,
    pub model: String,
}

#[derive(Default)]
pub struct EmbedState {
    child: Mutex<Option<Child>>,
    info: Mutex<Option<EmbedInfo>>,
}

fn pick_port(preferred: u16) -> u16 {
    for p in preferred..preferred.saturating_add(50) {
        if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return p;
        }
    }
    preferred
}

pub fn health_ok(port: u16) -> bool {
    ureq::get(&format!("http://127.0.0.1:{port}/health"))
        .timeout(std::time::Duration::from_millis(600))
        .call()
        .map(|r| r.status() == 200)
        .unwrap_or(false)
}

pub fn status(app: &AppHandle) -> Option<EmbedInfo> {
    let st = app.state::<EmbedState>();
    let mut guard = match st.child.lock() {
        Ok(g) => g,
        Err(_) => return None,
    };
    let alive = match guard.as_mut() {
        Some(child) => match child.try_wait() {
            Ok(Some(_)) => {
                *guard = None; // processo morreu
                false
            }
            _ => true,
        },
        None => false,
    };
    drop(guard);
    if !alive {
        if let Ok(mut i) = st.info.lock() {
            *i = None;
        }
        return None;
    }
    let info = st.info.lock().ok().and_then(|g| g.clone());
    info
}

pub fn stop(app: &AppHandle) {
    let st = app.state::<EmbedState>();
    let taken = st.child.lock().ok().and_then(|mut g| g.take());
    if let Some(mut child) = taken {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Ok(mut i) = st.info.lock() {
        *i = None;
    };
}

/// Sobe o servidor de embeddings para `model_path` (ou reusa se já roda com
/// o mesmo modelo). Retorna a porta efetiva.
pub fn start(app: &AppHandle, model_path: &str) -> Result<EmbedInfo, String> {
    if let Some(info) = status(app) {
        if info.model == model_path {
            return Ok(info);
        }
    }
    stop(app);
    let bin_dir =
        resolve_binaries_dir(app).ok_or_else(|| "Nao encontrei llama-server (binaries).".to_string())?;
    let port = pick_port(EMBED_PORT);
    let mut cmd = Command::new(bin_dir.join(LLAMA_SERVER_BIN));
    cmd.args([
        "-m",
        model_path,
        "--embedding",
        "--pooling",
        "mean",
        "-ngl",
        "0",
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ])
    .current_dir(&bin_dir)
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Falha ao iniciar o servidor de embeddings: {e}"))?;
    // logs do servidor de embedding vão para a aba Logs (diagnóstico)
    if let Some(err) = child.stderr.take() {
        let app2 = app.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(err);
            for line in reader.lines().map_while(Result::ok) {
                let _ = app2.emit("server-log", format!("[embed] {line}"));
            }
        });
    }
    let info = EmbedInfo {
        port,
        model: model_path.to_string(),
    };
    let st = app.state::<EmbedState>();
    *st.child.lock().map_err(|e| e.to_string())? = Some(child);
    *st.info.lock().map_err(|e| e.to_string())? = Some(info.clone());
    let _ = app.emit(
        "server-log",
        format!(
            "[embed] servidor de embeddings em 127.0.0.1:{port} (modelo {})",
            model_path
        ),
    );
    Ok(info)
}

/// Embeda um lote de textos. Espera o servidor ficar saudável (o modelo de
/// embedding carrega em segundos; o teto evita travar pra sempre).
pub fn embeddings(app: &AppHandle, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let info = status(app).ok_or_else(|| "Servidor de embeddings não está rodando".to_string())?;
    let mut healthy = false;
    for _ in 0..240 {
        if health_ok(info.port) {
            healthy = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    if !healthy {
        return Err("Servidor de embeddings não ficou saudável (modelo inválido?)".into());
    }
    let body = serde_json::json!({ "input": texts, "model": "local" });
    let resp = ureq::post(&format!("http://127.0.0.1:{}/v1/embeddings", info.port))
        .timeout(std::time::Duration::from_secs(300))
        .send_json(body)
        .map_err(|e| format!("embeddings: {e}"))?;
    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("embeddings json: {e}"))?;
    let arr = json
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "resposta sem 'data'".to_string())?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let vec = item
            .get("embedding")
            .and_then(|e| e.as_array())
            .ok_or_else(|| "item sem 'embedding'".to_string())?;
        out.push(
            vec.iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect::<Vec<f32>>(),
        );
    }
    Ok(out)
}

pub fn kill_on_exit(app: &AppHandle) {
    stop(app);
}
