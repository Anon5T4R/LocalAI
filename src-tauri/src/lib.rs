mod gguf;
mod hardware;
mod hub;
mod server;
mod tuner;

use gguf::ModelInfo;
use hardware::HardwareInfo;
use server::{RunningInfo, ServerState, StatusReport};
use tauri::{AppHandle, Manager, RunEvent};
use tuner::{LlamaConfig, Recommendation, TuneOverrides};

#[tauri::command]
fn get_hardware(app: AppHandle) -> HardwareInfo {
    let gpu = server::vulkan_gpu(&app);
    hardware::get_hardware(gpu)
}

#[tauri::command]
fn scan_models(dirs: Vec<String>) -> Vec<ModelInfo> {
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

/// Diretorios candidatos onde costumam existir modelos GGUF.
#[tauri::command]
fn default_model_dirs() -> Vec<String> {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Linux: webkit2gtk recente mostra TELA BRANCA com o renderer DMABUF em
    // varios drivers (tipico em AMD/Mesa). Desliga antes do GTK inicializar.
    // Tambem desliga o modo de compositing como reforco contra glitches/branco.
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
        // No Wayland o webkitgtk (ainda mais empacotado em AppImage) costuma
        // renderizar tela branca mesmo com o DMABUF desligado; forcar XWayland
        // (GDK_BACKEND=x11) e o remedio mais confiavel. So ativa o fallback se
        // estivermos em Wayland e o usuario nao tiver escolhido um backend.
        let on_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE")
                .map(|t| t.eq_ignore_ascii_case("wayland"))
                .unwrap_or(false);
        if on_wayland && std::env::var_os("GDK_BACKEND").is_none() {
            std::env::set_var("GDK_BACKEND", "x11");
        }
    }

    tauri::Builder::default()
        .manage(ServerState::default())
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
            load_conversations,
            save_conversations
        ])
        .build(tauri::generate_context!())
        .expect("erro ao inicializar o LocalAI Studio")
        .run(|app: &AppHandle, event| {
            if let RunEvent::ExitRequested { .. } = event {
                server::kill_on_exit(app);
            }
            if let RunEvent::Exit = event {
                server::kill_on_exit(app);
            }
        });
}
