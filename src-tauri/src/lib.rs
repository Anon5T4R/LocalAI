mod bench;
mod gguf;
mod hardware;
mod hub;
mod quant;
mod server;
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
fn bench_start(app: AppHandle, paths: Vec<String>) -> Result<(), String> {
    bench::start(&app, paths)
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
        .on_window_event(|window, event| {
            // Bug do tao <= 0.35 no GNOME/Wayland: botões da titlebar (min/
            // max/fechar) mortos até um resize (tauri#13440, tauri#11856). O
            // toggle de `resizable` em cada foco força o GTK a revalidar as
            // decorações, restaurando o estado original em seguida. Remover
            // quando o tauri puxar o tao 0.36 (via wry 0.56).
            #[cfg(target_os = "linux")]
            if let tauri::WindowEvent::Focused(true) = event {
                let r = window.is_resizable().unwrap_or(true);
                let _ = window.set_resizable(!r);
                let _ = window.set_resizable(r);
            }
            #[cfg(not(target_os = "linux"))]
            let _ = (window, event);
        })
        .manage(ServerState::default())
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
            save_conversations
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
