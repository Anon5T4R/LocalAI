// Auto-tuner: dado o hardware + o modelo GGUF, calcula os flags do
// llama-server que extraem o maximo da maquina detectada. As heuristicas
// assumem que geracao e memory-bound (banda de RAM como gargalo) e que
// prompt e compute-bound — vale para CPUs x86/ARM e APUs/iGPUs em geral.

use crate::gguf::ModelInfo;
use crate::hardware::HardwareInfo;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlamaConfig {
    pub model_path: String,
    pub model_name: String,
    pub ctx_size: u32,
    pub threads: usize,
    pub threads_batch: usize,
    pub batch: u32,
    pub ubatch: u32,
    pub n_gpu_layers: u32,
    pub mlock: bool,
    pub no_mmap: bool,
    /// "on" | "off" | "auto"
    pub flash_attn: String,
    /// "f16" | "q8_0" | "q4_0"
    pub cache_type_k: String,
    pub cache_type_v: String,
    pub mmproj: Option<String>,
    pub host: String,
    pub port: u16,
    /// Speculative decoding: modelo-rascunho (mesma familia/tokenizer).
    pub draft_model: Option<String>,
    pub draft_n_gpu_layers: u32,
    pub draft_max: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TuneOverrides {
    pub ctx_size: Option<u32>,
    pub gpu_offload: Option<bool>,
    pub n_gpu_layers: Option<u32>,
    pub kv_quant: Option<bool>,
    pub port: Option<u16>,
    /// Carregar o encoder de visao (mmproj). Desligado por padrao.
    pub use_mmproj: Option<bool>,
    /// Speculative decoding: caminho e tamanho do modelo-rascunho.
    pub use_speculative: Option<bool>,
    pub draft_path: Option<String>,
    pub draft_size_gb: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    pub config: LlamaConfig,
    pub rationale: Vec<String>,
    pub warnings: Vec<String>,
    pub est_ram_gb: f64,
    pub fits_in_ram: bool,
    pub max_gpu_layers: u32,
    pub gpu_recommended: bool,
}

// Estima a RAM do KV cache em GB.
// bytes = 2 (K e V) * n_layers * ctx * kv_embd * bytes_por_elem
// kv_embd ja considera GQA (n_kv_heads * head_dim), bem menor que embedding.
fn estimate_kv_gb(ctx: u32, block_count: Option<u32>, kv_embd: u32, bytes_per_elem: f64) -> f64 {
    let layers = block_count.unwrap_or(32) as f64;
    let embd = kv_embd as f64;
    let bytes = 2.0 * layers * (ctx as f64) * embd * bytes_per_elem;
    bytes / 1e9
}

// Dimensao efetiva do KV considerando GQA: embedding * (head_count_kv / head_count).
fn kv_embd_of(model: &ModelInfo) -> u32 {
    let embd = model.embedding_length.unwrap_or(4096);
    match (model.head_count, model.head_count_kv) {
        (Some(h), Some(hkv)) if h > 0 => {
            ((embd as u64 * hkv as u64) / h as u64) as u32
        }
        _ => embd,
    }
}

fn kv_bytes_per_elem(kv_quant: bool) -> (f64, &'static str) {
    if kv_quant {
        (1.0, "q8_0") // ~1 byte/elemento
    } else {
        (2.0, "f16")
    }
}

pub fn recommend(
    hw: &HardwareInfo,
    model: &ModelInfo,
    ov: &TuneOverrides,
) -> Recommendation {
    let mut rationale: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // --- Threads ---
    let threads = hw.recommended_gen_threads;
    let threads_batch = hw.recommended_batch_threads;
    rationale.push(format!(
        "Threads de geracao = {} (nucleos fisicos). Em geracao o gargalo e a banda de memoria; usar os {} threads logicos (SMT) normalmente piora por contencao.",
        threads, hw.logical_cores
    ));
    rationale.push(format!(
        "Threads de prompt (batch) = {} (todos os logicos). Processar o prompt e compute-bound e escala com mais threads.",
        threads_batch
    ));

    // --- ISA ---
    rationale.push(format!(
        "SIMD detectado: {}. O llama.cpp seleciona a DLL ggml-cpu correspondente em runtime.",
        hw.features.best_cpu_isa
    ));

    // --- Contexto ---
    let model_ctx_max = model.context_length.unwrap_or(8192);
    let ctx = ov
        .ctx_size
        .unwrap_or(8192)
        .min(model_ctx_max.max(2048));
    rationale.push(format!(
        "Contexto = {} tokens (teto do modelo: {}). Contexto maior = mais RAM de KV cache e geracao mais lenta no fim do contexto.",
        ctx, model_ctx_max
    ));

    // --- KV cache quant ---
    let kv_quant = ov.kv_quant.unwrap_or(false);
    let (bpe, kv_label) = kv_bytes_per_elem(kv_quant);
    if kv_quant {
        rationale.push(
            "KV cache em q8_0: ~metade da RAM e da banda do KV, ajudando em contextos longos nesta CPU memory-bound (custo: leve perda de qualidade).".to_string(),
        );
    } else {
        rationale.push(
            "KV cache em f16 (qualidade). Ative o KV quantizado para ganhar RAM/velocidade em contextos longos.".to_string(),
        );
    }

    // --- KV cache (RAM) ---
    let kv_gb = estimate_kv_gb(ctx, model.block_count, kv_embd_of(model), bpe);

    // --- Speculative decoding (modelo-rascunho) ---
    // Opt-in: no benchmark deste hardware (rascunho na CPU, prompt aberto) o
    // speculative ficou neutro/levemente pior. Fica como toggle para testar em
    // workloads previsiveis (codigo) ou com rascunho na GPU + RAM livre.
    let draft_path = ov.draft_path.clone();
    let use_speculative = draft_path.is_some() && ov.use_speculative.unwrap_or(false);
    let draft_size = if use_speculative {
        ov.draft_size_gb.unwrap_or(1.0)
    } else {
        0.0
    };

    // --- GPU offload (Vulkan) ---
    // Em iGPUs/APUs (memoria compartilhada, UMA): offload TOTAL costuma render
    // bem mais que CPU puro, mas offload PARCIAL costuma render MENOS (overhead
    // de split + banda compartilhada) — e "tudo-ou-nada". Em GPUs dedicadas,
    // offload parcial e um meio-termo valido quando o modelo nao cabe na VRAM.
    let gpu_label = hw
        .gpu_name
        .clone()
        .unwrap_or_else(|| "GPU (Vulkan)".to_string());
    let is_igpu = hw.gpu_is_igpu;
    let max_gpu_layers = model.block_count.map(|b| b + 1).unwrap_or(0);
    // Orcamento real da GPU (detectado via Vulkan), com fallback ~metade da RAM.
    let gpu_budget_gb = hw.gpu_budget_gb;
    // o rascunho fica na CPU (RAM normal), entao nao pesa no orcamento da GPU
    let fits_gpu = max_gpu_layers > 0 && (model.size_gb + kv_gb) < gpu_budget_gb * 0.92;
    let gpu_recommended = fits_gpu;
    let gpu_on = ov.gpu_offload.unwrap_or(fits_gpu);
    let n_gpu_layers = if gpu_on {
        let n = ov.n_gpu_layers.unwrap_or(max_gpu_layers).min(max_gpu_layers);
        if n >= max_gpu_layers {
            rationale.push(format!(
                "Offload Vulkan TOTAL: {} camadas em {} (o modelo + KV cabem no orcamento de ~{:.1} GB da GPU). Com tudo na GPU, evita o overhead de dividir o grafo entre CPU e GPU.",
                n, gpu_label, gpu_budget_gb
            ));
        } else {
            rationale.push(format!(
                "Offload Vulkan PARCIAL: {} de {} camadas em {}.",
                n, max_gpu_layers, gpu_label
            ));
            if is_igpu {
                warnings.push(
                    "Em GPUs integradas (memoria compartilhada com a CPU), offload PARCIAL costuma render MENOS que CPU puro. Prefira TOTAL (todas as camadas) ou ngl=0.".to_string(),
                );
            }
        }
        n
    } else {
        if fits_gpu {
            rationale.push(format!(
                "CPU puro (ngl=0) por sua escolha. Dica: este modelo cabe na {} — offload TOTAL tende a ser mais rapido.",
                gpu_label
            ));
        } else if is_igpu {
            rationale.push(format!(
                "CPU puro (ngl=0): o modelo ({:.1} GB) nao cabe no orcamento da GPU (~{:.1} GB) e, em GPUs integradas, offload parcial costuma render menos que CPU puro.",
                model.size_gb, gpu_budget_gb
            ));
        } else {
            rationale.push(format!(
                "CPU puro (ngl=0): o modelo ({:.1} GB) nao cabe inteiro na VRAM (~{:.1} GB). Em GPU dedicada, vale testar offload parcial no controle acima.",
                model.size_gb, gpu_budget_gb
            ));
        }
        0
    };

    // --- Estimativa de RAM ---
    // draft_size entra aqui porque o rascunho fica na RAM da CPU.
    let overhead_gb = 0.8;
    let est_ram_gb = model.size_gb + kv_gb + draft_size + overhead_gb;
    let fits_in_ram = est_ram_gb < hw.total_ram_gb * 0.92;

    rationale.push(format!(
        "RAM estimada: {:.1} GB = modelo {:.1} GB + KV {} {:.1} GB + overhead {:.1} GB (total da maquina: {:.1} GB).",
        est_ram_gb, model.size_gb, kv_label, kv_gb, overhead_gb, hw.total_ram_gb
    ));
    if !fits_in_ram {
        warnings.push(format!(
            "A estimativa ({:.1} GB) esta perto/acima da RAM total ({:.1} GB). Reduza o contexto, use KV q8_0 ou uma quantizacao menor para evitar swap (que mata o desempenho).",
            est_ram_gb, hw.total_ram_gb
        ));
    }

    // --- mlock / mmap ---
    // CRITICO: mlock SO no modo CPU. Com offload em iGPU o modelo vai para o
    // GTT (mesma RAM fisica); travar a copia da CPU dobra o uso de RAM e
    // trava o carregamento (medido: timeout vs 8s sem mlock).
    let mlock = n_gpu_layers == 0 && fits_in_ram && est_ram_gb < hw.total_ram_gb * 0.72;
    if mlock {
        rationale.push(
            "--mlock ligado: trava o modelo na RAM e impede o sistema de paginar para o disco (evita engasgos). So no modo CPU, onde sobra RAM.".to_string(),
        );
    } else if n_gpu_layers > 0 {
        rationale.push(
            "--mlock desligado: com offload, o modelo vai para a memoria da GPU (em iGPUs, a mesma RAM fisica via GTT). Travar tambem a copia da CPU dobraria o uso de memoria.".to_string(),
        );
    } else {
        rationale.push(
            "--mlock desligado: a margem de RAM esta apertada para travar tudo na memoria.".to_string(),
        );
    }

    // --- Flash attention ---
    let flash_attn = "on".to_string();
    rationale.push(
        "Flash attention = on: reduz a RAM do KV cache e costuma acelerar; suportado no backend de CPU.".to_string(),
    );

    // --- Batch / ubatch ---
    let batch = 2048u32;
    let ubatch = 512u32;
    rationale.push(format!(
        "batch={} / ubatch={}: bom equilibrio de throughput de prompt sem estourar memoria nesta classe de CPU.",
        batch, ubatch
    ));

    // --- Visao (mmproj) opt-in ---
    let use_mmproj = ov.use_mmproj.unwrap_or(false);
    let mmproj = if use_mmproj {
        model.mmproj_path.clone()
    } else {
        None
    };
    if model.mmproj_path.is_some() {
        if use_mmproj {
            rationale.push(
                "Visao (mmproj) LIGADA: carrega o encoder de imagens (+RAM e +tempo de load).".to_string(),
            );
        } else {
            rationale.push(
                "Visao (mmproj) desligada por padrao: carrega mais rapido e leve. Ligue se for enviar imagens.".to_string(),
            );
        }
    }

    // O rascunho (pequeno) fica SEMPRE na CPU: preserva a memoria da GPU para
    // o modelo grande e evita dobrar o uso de memoria (que travava o load).
    let draft_n_gpu_layers = 0u32;
    if use_speculative {
        rationale.push(
            "Speculative decoding LIGADO: o rascunho (na CPU) propoe ate 16 tokens por passo e o modelo grande verifica todos numa unica leitura de memoria. Como o gargalo da geracao e banda de memoria, isso tende a acelerar sem perder qualidade.".to_string(),
        );
        warnings.push(
            "Ganho do speculative depende da taxa de aceitacao do rascunho (alto em texto previsivel/codigo, menor em conteudo muito aberto).".to_string(),
        );
    }

    let port = ov.port.unwrap_or(8080);

    let config = LlamaConfig {
        model_path: model.path.clone(),
        model_name: model.name.clone(),
        ctx_size: ctx,
        threads,
        threads_batch,
        batch,
        ubatch,
        n_gpu_layers,
        mlock,
        no_mmap: false,
        flash_attn,
        cache_type_k: kv_label.to_string(),
        cache_type_v: kv_label.to_string(),
        mmproj,
        host: "127.0.0.1".to_string(),
        port,
        draft_model: if use_speculative { draft_path } else { None },
        draft_n_gpu_layers,
        draft_max: 16,
    };

    Recommendation {
        config,
        rationale,
        warnings,
        est_ram_gb,
        fits_in_ram,
        max_gpu_layers,
        gpu_recommended,
    }
}
