mod bench;
mod embed;
mod gguf;
mod hardware;
mod hub;
mod ocr;
mod quant;
mod rag;
mod server;
mod tools;
mod tuner;

use gguf::ModelInfo;
use hardware::HardwareInfo;
use server::{RunningInfo, ServerState, StatusReport};
use tauri::{AppHandle, Manager, RunEvent};
use tuner::{LlamaConfig, Recommendation, TuneOverrides};

// async: a detecção de GPU roda `llama-server --list-devices` (subprocesso) na 1ª
// chamada; síncrono isso bloqueava a main thread e congelava a janela ao abrir.
#[tauri::command]
async fn get_hardware(app: AppHandle) -> HardwareInfo {
    let gpu = server::vulkan_gpu(&app);
    hardware::get_hardware(gpu)
}

// async: varre diretórios e lê headers GGUF — fora da main thread.
#[tauri::command]
async fn scan_models(dirs: Vec<String>) -> Vec<ModelInfo> {
    gguf::scan_dirs(&dirs)
}

#[tauri::command]

fn recommend_config(
    app: AppHandle,
    model: ModelInfo,
    overrides: TuneOverrides,
) -> Recommendation {
    let gpu = server::vulkan_gpu(&app);
    let hw = hardware::get_hardware(gpu);
    tuner::recommend(&hw, &model, &overrides)
}

// ---------- Hugging Face Hub ----------

#[tauri::command]
fn hf_search(query: String) -> Result<Vec<hub::HubModel>, String> {
    hub::search(&query)
}

#[tauri::command]
fn hf_list_files(repo: String) -> Result<Vec<hub::HubFile>, String> {
    hub::list_files(&repo)
}

#[tauri::command]
fn hf_download(
    app: AppHandle,
    repo: String,
    file: String,
    dest_dir: String,
) -> Result<(), String> {
    hub::download(app, repo, file, dest_dir)
}

#[tauri::command]
fn hf_cancel_download() {
    hub::cancel();
}

// ---------- Quantizacao de GGUF (llama-quantize) ----------

#[tauri::command]
fn quant_start(app: AppHandle, input: String, out_type: String) -> Result<String, String> {
    quant::start(&app, input, out_type)
}

#[tauri::command]
fn quant_cancel(app: AppHandle) {
    quant::cancel(&app);
}

// ---------- Comparador de modelos (llama-bench) ----------

#[tauri::command]
fn bench_start(app: AppHandle, paths: Vec<String>, cpu_gpu: bool) -> Result<(), String> {
    bench::start(&app, paths, cpu_gpu)
}

#[tauri::command]
fn bench_cancel(app: AppHandle) {
    bench::cancel(&app);
}

// ---------- Persistencia de conversas (arquivo no app_data_dir) ----------
// localStorage do WebView pode ser limpo pelo sistema; arquivo e mais seguro.

fn conversations_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Sem diretorio de dados do app: {e}"))?;
    Ok(dir.join("conversations.json"))
}

#[tauri::command]
fn load_conversations(app: AppHandle) -> Option<String> {
    let path = conversations_path(&app).ok()?;
    std::fs::read_to_string(path).ok()
}

#[tauri::command]
fn save_conversations(app: AppHandle, json: String) -> Result<(), String> {
    let path = conversations_path(&app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Nao criei {dir:?}: {e}"))?;
    }
    std::fs::write(&path, json).map_err(|e| format!("Nao gravei as conversas: {e}"))
}

#[tauri::command]
fn start_server(app: AppHandle, config: LlamaConfig) -> Result<RunningInfo, String> {
    server::start(&app, config)
}

#[tauri::command]
fn stop_server(app: AppHandle) -> Result<(), String> {
    server::stop(&app)
}

#[tauri::command]
fn server_status(app: AppHandle) -> StatusReport {
    server::status(&app)
}

#[tauri::command]
fn pick_folder() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Escolha a pasta com modelos GGUF")
        .pick_folder()
        .map(|p| p.to_string_lossy().into_owned())
}

// ---------- RAG local (PDF por conversa) ----------
// Todos async + spawn_blocking: extração de PDF e busca nunca rodam na main
// thread (o gotcha da suíte: comando síncrono congela a janela).

#[tauri::command]
async fn rag_add_pdf(
    app: AppHandle,
    path: String,
    conv_id: String,
) -> Result<rag::AddDocResult, String> {
    tauri::async_runtime::spawn_blocking(move || rag::ingest_pdf(&app, &path, &conv_id))
        .await
        .map_err(|e| format!("ingest: {e}"))?
}

#[tauri::command]
async fn rag_search(
    app: AppHandle,
    conv_id: String,
    query: String,
    k: u32,
    hybrid: Option<bool>,
) -> Result<Vec<rag::Chunk>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = rag::open_db(&app)?;
        let want_hybrid = hybrid.unwrap_or(false);
        // BM25 (léxico) sempre roda; no híbrido ele vira o ranking de 20
        let bm25 = rag::search(&conn, &conv_id, &query, if want_hybrid { 20 } else { k })?;
        if !want_hybrid {
            return Ok(bm25);
        }
        // semântico: só se o servidor de embeddings está de pé; qualquer
        // falha (modelo ruim, servidor fora) cai de volta pro BM25 puro
        if embed::status(&app).is_none() {
            return Ok(bm25.into_iter().take(k as usize).collect());
        }
        let qv = match embed::embeddings(&app, &[query]) {
            Ok(v) => match v.into_iter().next() {
                Some(v) => v,
                None => return Ok(bm25.into_iter().take(k as usize).collect()),
            },
            Err(_) => return Ok(bm25.into_iter().take(k as usize).collect()),
        };
        let all = rag::all_vecs(&conn, &conv_id)?;
        let mut scored: Vec<(f32, String, u32)> = all
            .iter()
            .map(|(d, s, v)| (rag::cosine(&qv, v), d.clone(), *s))
            .filter(|(score, _, _)| *score > 0.0)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(20);
        let mut sem = Vec::new();
        for (_, doc_id, seq) in scored {
            if let Some(c) = rag::chunk_by_key(&conn, &conv_id, &doc_id, seq)? {
                sem.push(c);
            }
        }
        Ok(rag::rrf_fuse(&bm25, &sem, k as usize))
    })
    .await
    .map_err(|e| format!("search: {e}"))?
}

/// Embeda os chunks ainda sem vetor desta conversa (busca semântica).
/// Sobe o servidor de embeddings lazy, na primeira vez.
#[tauri::command]
async fn rag_embed_pending(
    app: AppHandle,
    conv_id: String,
    model_path: String,
) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = rag::open_db(&app)?;
        let pending = rag::pending_chunks(&conn, &conv_id)?;
        if pending.is_empty() {
            return Ok(0);
        }
        embed::start(&app, &model_path)?;
        let mut done = 0usize;
        // lotes de 16: streaming no server + progresso previsível
        for batch in pending.chunks(16) {
            let texts: Vec<String> = batch.iter().map(|(_, _, t)| t.clone()).collect();
            let vecs = embed::embeddings(&app, &texts)?;
            for (i, (doc_id, seq, _)) in batch.iter().enumerate() {
                if let Some(v) = vecs.get(i) {
                    rag::add_vec(&conn, &conv_id, doc_id, *seq, v)?;
                    done += 1;
                }
            }
        }
        Ok(done)
    })
    .await
    .map_err(|e| format!("embed: {e}"))?
}

#[tauri::command]
async fn rag_doc_text(
    app: AppHandle,
    conv_id: String,
    doc_id: String,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = rag::open_db(&app)?;
        rag::doc_text(&conn, &conv_id, &doc_id)
    })
    .await
    .map_err(|e| format!("doc_text: {e}"))?
}

#[tauri::command]
async fn rag_remove_doc(app: AppHandle, conv_id: String, doc_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = rag::open_db(&app)?;
        rag::remove_doc(&conn, &conv_id, &doc_id)
    })
    .await
    .map_err(|e| format!("remove_doc: {e}"))?
}

#[tauri::command]
async fn rag_remove_conv(app: AppHandle, conv_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = rag::open_db(&app)?;
        rag::remove_conv(&conn, &conv_id)
    })
    .await
    .map_err(|e| format!("remove_conv: {e}"))?
}

/// Diálogo nativo de escolha do PDF (o backend precisa do PATH do arquivo —
/// o input HTML do WebView só entrega bytes, sem caminho).
#[tauri::command]
fn pick_pdf() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Escolha o PDF")
        .add_filter("PDF", &["pdf"])
        .pick_file()
        .map(|p| p.to_string_lossy().into_owned())
}

// ---------- Ferramentas do modelo (tool calls) ----------

#[tauri::command]
fn tool_save_document(
    dir: String,
    filename: String,
    format: String,
    content: String,
) -> Result<tools::SaveResult, String> {
    tools::save_document(&dir, &filename, &format, &content)
}

#[tauri::command]
fn open_in_folder(path: String) -> Result<(), String> {
    tools::open_in_folder(&path)
}

#[tauri::command]
fn default_save_dir() -> String {
    tools::default_save_dir()
}

/// Diretorios candidatos onde costumam existir modelos GGUF.
/// async: proba unidades A-Z com .exists() — drive de rede desconectado pode travar segundos.
#[tauri::command]
async fn default_model_dirs() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push_if_exists = |p: std::path::PathBuf| {
        if p.exists() {
            out.push(p.to_string_lossy().into_owned());
        }
    };

    // Pastas do LM Studio em qualquer unidade montada (so Windows)
    #[cfg(windows)]
    for drive in 'A'..='Z' {
        let root = std::path::PathBuf::from(format!("{drive}:\\"));
        if !root.exists() {
            continue;
        }
        push_if_exists(root.join("LocalAIModels").join(".lmstudio").join("hub").join("models"));
    }
    // home: USERPROFILE no Windows, HOME no Linux/macOS
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = std::path::PathBuf::from(home);
        push_if_exists(home.join(".lmstudio").join("models"));
        push_if_exists(home.join(".lmstudio").join("hub").join("models"));
        push_if_exists(home.join(".cache").join("lm-studio").join("models"));
        push_if_exists(
            home.join(".cache")
                .join("huggingface")
                .join("hub"),
        );
        // pasta propria do LocalAI (destino padrao dos downloads)
        push_if_exists(home.join("LocalAI").join("models"));
        // pasta da era TaylorAI (compat: modelos baixados antes do rebrand)
        push_if_exists(home.join("TaylorAI").join("models"));
    }
    out
}

/// Pasta padrao para salvar downloads: a primeira pasta de modelos do usuario
/// ou ~/LocalAI/models (criada na hora).
#[tauri::command]
fn default_download_dir() -> String {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = home.join("LocalAI").join("models");
    let _ = std::fs::create_dir_all(&dir);
    dir.to_string_lossy().into_owned()
}

/// Contorna a titlebar quebrada do tao <= 0.35 no GNOME/Wayland (CSD propia
/// com regiao de input morta — causa e fix em tao#1218, so via tauri 2.12):
/// troca por uma HeaderBar comum com layout forcado min/max/fechar, ANTES do
/// primeiro map. Sai junto com o upgrade ao tao 0.36 (wry 0.56).
#[cfg(target_os = "linux")]
fn instalar_csd_limpa(w: &tauri::WebviewWindow) {
    use gtk::prelude::*;
    let Ok(gw) = w.gtk_window() else { return };
    let header = gtk::HeaderBar::new();
    header.set_show_close_button(true);
    header.set_decoration_layout(Some("menu:minimize,maximize,close"));
    header.set_title(Some("LocalAI Studio"));
    header.show();
    gw.set_titlebar(Some(&header));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ── Contorno da tela branca do webkit: REMOVIDO, e o porquê importa ──────
    //
    // Este bloco desligava o renderer DMABUF, desligava o compositing e forçava
    // XWayland, porque o webkit2gtk pintava a janela inteira de branco em
    // Arch/GNOME. Era mitigação às cegas — o comentário dizia "branco é pior que
    // lento" — e custava a aceleração do WebView.
    //
    // A CAUSA foi encontrada em 26/07/2026 e é de EMPACOTAMENTO, não de código:
    // o AppDir do AppImage levava `libwayland-*` do Ubuntu do CI, que brigavam
    // com o Mesa do host e derrubavam o EGL (`EGL_BAD_PARAMETER`). Corrigido em
    // `Anon5T4R/linux-packaging`: as libs que falam com driver/compositor agora
    // vêm do host, e o pacote nativo (pacman/apt) usa o webkit do sistema.
    // Tratar o sintoma deixou de fazer sentido.
    //
    // Remover o forçamento NÃO tira a saída de emergência: estas variáveis são
    // lidas pelo próprio webkitgtk, não por este código. Se a tela branca voltar
    // em alguma combinação de driver, rodar com
    // `WEBKIT_DISABLE_DMABUF_RENDERER=1` continua funcionando — e aí é sinal de
    // que sobrou lib de host em algum AppDir, que é onde se deve olhar.

    tauri::Builder::default()
        .setup(|app| {
            // Titlebar limpa no GNOME/Wayland (tao <= 0.35) — ver lib.rs do LocalImage.
            #[cfg(target_os = "linux")]
            if let Some(w) = app.get_webview_window("main") {
                instalar_csd_limpa(&w);
            }
            Ok(())
        })
        .manage(ServerState::default())
        .manage(embed::EmbedState::default())
        .manage(quant::QuantState::default())
        .manage(bench::BenchState::default())
        .invoke_handler(tauri::generate_handler![
            get_hardware,
            scan_models,
            recommend_config,
            start_server,
            stop_server,
            server_status,
            pick_folder,
            default_model_dirs,
            default_download_dir,
            hf_search,
            hf_list_files,
            hf_download,
            hf_cancel_download,
            quant_start,
            quant_cancel,
            bench_start,
            bench_cancel,
            load_conversations,
            save_conversations,
            pick_pdf,
            rag_add_pdf,
            rag_search,
            rag_doc_text,
            rag_remove_doc,
            rag_remove_conv,
            tool_save_document,
            open_in_folder,
            default_save_dir
        ])
        .build(tauri::generate_context!())
        .expect("erro ao inicializar o LocalAI Studio")
        .run(|app: &AppHandle, event| {
            if let RunEvent::ExitRequested { .. } = event {
                server::kill_on_exit(app);
                quant::kill_on_exit(app);
                bench::kill_on_exit(app);
            }
            if let RunEvent::Exit = event {
                server::kill_on_exit(app);
                quant::kill_on_exit(app);
                bench::kill_on_exit(app);
            }
        });
}
