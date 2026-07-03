// Integracao com o Hugging Face Hub: busca de repositorios GGUF, listagem
// dos arquivos .gguf de um repo e download com progresso via eventos Tauri.
// Tudo via ureq (Rust) para nao depender de CORS no WebView.

use serde::Serialize;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

static CANCEL: AtomicBool = AtomicBool::new(false);
static DOWNLOADING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Serialize)]
pub struct HubModel {
    pub id: String,
    pub downloads: u64,
    pub likes: u64,
    pub updated: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct HubFile {
    pub path: String,
    pub size_bytes: u64,
    pub size_gb: f64,
}

#[derive(Clone, Serialize)]
struct Progress {
    file: String,
    downloaded: u64,
    total: Option<u64>,
}

#[derive(Clone, Serialize)]
struct DoneEvent {
    file: String,
    path: Option<String>,
    error: Option<String>,
}

fn encode(q: &str) -> String {
    let mut out = String::new();
    for b in q.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn search(query: &str) -> Result<Vec<HubModel>, String> {
    let url = format!(
        "https://huggingface.co/api/models?search={}&filter=gguf&sort=downloads&direction=-1&limit=25",
        encode(query)
    );
    let resp = ureq::get(&url)
        .timeout(Duration::from_secs(20))
        .call()
        .map_err(|e| format!("Busca no Hugging Face falhou: {e}"))?;
    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Resposta invalida do Hugging Face: {e}"))?;
    let arr = json.as_array().cloned().unwrap_or_default();
    Ok(arr
        .iter()
        .filter_map(|m| {
            Some(HubModel {
                id: m.get("id")?.as_str()?.to_string(),
                downloads: m.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0),
                likes: m.get("likes").and_then(|v| v.as_u64()).unwrap_or(0),
                updated: m
                    .get("lastModified")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            })
        })
        .collect())
}

pub fn list_files(repo: &str) -> Result<Vec<HubFile>, String> {
    // recursive=true: muitos repos guardam os GGUF em subpastas por quant
    let url = format!(
        "https://huggingface.co/api/models/{repo}/tree/main?recursive=true"
    );
    let resp = ureq::get(&url)
        .timeout(Duration::from_secs(20))
        .call()
        .map_err(|e| format!("Falha ao listar arquivos de {repo}: {e}"))?;
    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Resposta invalida do Hugging Face: {e}"))?;
    let arr = json.as_array().cloned().unwrap_or_default();
    let mut files: Vec<HubFile> = arr
        .iter()
        .filter_map(|f| {
            let path = f.get("path")?.as_str()?.to_string();
            if !path.to_lowercase().ends_with(".gguf") {
                return None;
            }
            let size = f.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            Some(HubFile {
                path,
                size_bytes: size,
                size_gb: size as f64 / 1e9,
            })
        })
        .collect();
    files.sort_by(|a, b| a.size_bytes.cmp(&b.size_bytes));
    Ok(files)
}

pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Dispara o download numa thread propria e retorna na hora; o progresso sai
/// nos eventos "hub-progress" e a conclusao (ou erro) em "hub-done".
pub fn download(app: AppHandle, repo: String, file: String, dest_dir: String) -> Result<(), String> {
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Err("Ja existe um download em andamento.".into());
    }
    CANCEL.store(false, Ordering::SeqCst);

    std::thread::spawn(move || {
        let result = do_download(&app, &repo, &file, &dest_dir);
        DOWNLOADING.store(false, Ordering::SeqCst);
        let _ = app.emit(
            "hub-done",
            DoneEvent {
                file: file.clone(),
                path: result.as_ref().ok().cloned(),
                error: result.err(),
            },
        );
    });
    Ok(())
}

fn do_download(app: &AppHandle, repo: &str, file: &str, dest_dir: &str) -> Result<String, String> {
    let url = format!("https://huggingface.co/{repo}/resolve/main/{file}");
    // arquivos em subpasta (Q4_K_M/model.gguf) viram so o nome final no disco
    let fname = file.rsplit('/').next().unwrap_or(file).to_string();

    let dir = PathBuf::from(dest_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Nao criei a pasta destino: {e}"))?;
    let final_path = dir.join(&fname);
    let part_path = dir.join(format!("{fname}.part"));

    // timeout so de conexao/leitura por bloco — nunca timeout total (arquivos de GBs)
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .build();
    let resp = agent
        .get(&url)
        .call()
        .map_err(|e| format!("Download falhou: {e}"))?;
    let total: Option<u64> = resp
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok());

    let mut reader = resp.into_reader();
    let mut out = std::fs::File::create(&part_path)
        .map_err(|e| format!("Nao criei o arquivo: {e}"))?;

    let mut buf = [0u8; 256 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_emit = Instant::now();
    loop {
        if CANCEL.load(Ordering::SeqCst) {
            drop(out);
            let _ = std::fs::remove_file(&part_path);
            return Err("Download cancelado.".into());
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Erro lendo o download: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| format!("Erro gravando no disco: {e}"))?;
        downloaded += n as u64;
        if last_emit.elapsed() >= Duration::from_millis(300) {
            last_emit = Instant::now();
            let _ = app.emit(
                "hub-progress",
                Progress {
                    file: file.to_string(),
                    downloaded,
                    total,
                },
            );
        }
    }
    out.flush().ok();
    drop(out);

    if let Some(t) = total {
        if downloaded < t {
            let _ = std::fs::remove_file(&part_path);
            return Err(format!(
                "Download incompleto ({downloaded} de {t} bytes)."
            ));
        }
    }
    std::fs::rename(&part_path, &final_path)
        .map_err(|e| format!("Nao renomeei o .part: {e}"))?;
    Ok(final_path.to_string_lossy().into_owned())
}
