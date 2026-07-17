// i18n leve e VANILLA (sem React) do LocalAI Studio. `pt` é a fonte da verdade
// das chaves; `en`/`es` como `Record<MessageKey, string>` fazem o compilador
// recusar chave faltando ou sobrando (paridade garantida por tsc).
//
// Reatividade por RELOAD: o app monta toda a UI imperativamente no boot
// (buildShell etc.), então `setLocale` persiste no localStorage e chama
// `location.reload()` — no próximo carregamento tudo re-renderiza com `t()`
// fresco. Não há store/hook: `t()` roda em qualquer lugar (fora de componente).

export type Locale = "pt" | "en" | "es";

/** Endônimos (o próprio idioma no idioma dele) — NÃO traduzir. */
export const LOCALE_LABELS: Record<Locale, string> = {
  pt: "Português",
  en: "English",
  es: "Español",
};

const LOCALE_KEY = "localai.locale";

const pt = {
  // ----- Sidebar / marca -----
  "brand.sub": "GGUF na CPU/iGPU",
  "sidebar.hwDetecting": "Detectando hardware…",
  "sidebar.folders": "Pastas de modelos",
  "sidebar.addFolder": "+ pasta",
  "sidebar.models": "Modelos GGUF",
  "sidebar.foldersEmpty": "Nenhuma pasta. Adicione uma.",

  // ----- Topbar -----
  "topbar.noModel": "Nenhum modelo selecionado",

  // ----- Abas -----
  "tab.chat": "Chat",
  "tab.tuner": "Ajustes & Auto-tuner",
  "tab.hub": "Baixar modelos",
  "tab.logs": "Logs",

  // ----- Status (pill) -----
  "status.stopped": "parado",
  "status.starting": "subindo",
  "status.ready": "pronto",
  "status.error": "erro",
  "status.loadModel": "carregue um modelo",

  // ----- Botões -----
  "btn.load": "Carregar",
  "btn.starting": "Subindo…",
  "btn.stop": "Parar",
  "btn.send": "Enviar",

  // ----- Card de hardware -----
  "hw.cores": "Núcleos",
  "hw.coresVal": "{phys} fís / {log} lóg",
  "hw.backend": "Backend",
  "hw.threads": "Tuner: {gen} threads p/ geração, {batch} p/ prompt",
  "hw.gpu": "GPU (Vulkan): {name} — ~{gb} GB p/ offload",
  "hw.gpuNone": "GPU Vulkan não detectada — estimando ~{gb} GB p/ offload",

  // ----- Lista de modelos -----
  "models.scanning": "Procurando…",
  "models.none": "Nenhum .gguf encontrado.",
  "model.vision": "👁 visão",

  // ----- Tuner -----
  "tuner.selectModel": "Selecione um modelo.",
  "tuner.controls": "Controles",
  "tuner.ctxTokens": "Contexto (tokens)",
  "tuner.gpuOffload": "Offload total na GPU ({gpu})",
  "tuner.recommended": " — recomendado",
  "tuner.gpuLayers": "Camadas na GPU ({n}/{max}) — parcial costuma piorar",
  "tuner.kvQuant": "KV cache quantizado (q8_0)",
  "tuner.vision": "Visão / multimodal (mmproj) — mais lento",
  "tuner.speculative": "Decodificação especulativa (rascunho: {name})",
  "tuner.port": "Porta",
  "tuner.resultConfig": "Configuração resultante",
  "tuner.fitsRam": "cabe na RAM",
  "tuner.nearRamLimit": "perto do limite de RAM",
  "tuner.ramOf": " (de {total} GB)",
  "tuner.cmdLine": "Linha de comando:",
  "tuner.why": "Por que essas escolhas (para o seu hardware)",
  "tuner.warnings": "Avisos",
  "flag.threadsGen": "threads (geração)",
  "flag.threadsPrompt": "threads (prompt)",
  "flag.ctx": "contexto",

  // ----- Valores comuns -----
  "common.yes": "sim",
  "common.no": "não",
  "common.on": "ligado",
  "common.off": "desligado",

  // ----- Logs / erros de servidor -----
  "log.loading": "\n=== Carregando {model} ===",
  "log.portAdjusted": "[localai] porta ajustada para {port}",
  "log.errorPrefix": "ERRO: {e}",
  "log.serverReady": "[localai] servidor sinalizou pronto",
  "err.processEnded": "processo encerrou durante o carregamento",
  "err.healthyTimeout": "timeout esperando o servidor ficar saudável",
  "err.serverResponded": "Servidor respondeu {status}: {text}",

  // ----- Chat -----
  "chat.empty":
    "Carregue um modelo e comece a conversar. As métricas de tok/s aparecem no topo.",
  "chat.newChat": "＋ Novo chat",
  "chat.sampling": "Amostragem & system prompt",
  "samp.temperature": "Temperatura",
  "chat.sysPlaceholder": "System prompt (opcional) — vazio por padrão",
  "chat.thinkMode":
    "Modo raciocínio (pensar) — desligado por padrão (aplica na hora, sem reiniciar)",
  "chat.attachTitle":
    "Anexar imagem (requer modelo com visão carregado com mmproj ligado no tuner)",
  "chat.inputPlaceholder":
    "Escreva sua mensagem…  (Enter envia, Shift+Enter quebra linha)",
  "chat.noVisionWarn":
    "⚠ o modelo atual está sem visão — recarregue com “Visão / multimodal (mmproj)” ligado no tuner",
  "conv.newChat": "Novo chat",
  "role.you": "Você",
  "role.assistant": "Assistente",
  "chat.copy": "copiar",
  "chat.copyTitle": "Copiar resposta",
  "chat.copied": "✓ copiado",
  "chat.thinking": "💭 Pensando…",
  "chat.thoughtOnly":
    "(o modelo respondeu apenas no canal de pensamento — abra 'Pensando' acima)",
  "chat.errorInline": "\n\n[erro: {e}]",
  "chat.image": "Imagem",
  "chat.describeImage": "Descreva a imagem.",

  // ----- Contador de contexto -----
  "ctx.title": "Tokens de contexto usados",
  "ctx.titleFull": "Tokens de contexto usados (prompt + resposta)",
  "ctx.fullWarn":
    "Contexto quase cheio: o servidor vai truncar as mensagens antigas. Aumente o contexto no tuner ou comece um chat novo.",

  // ----- Hub (baixar modelos) -----
  "hub.searchPlaceholder":
    "Buscar modelos GGUF no Hugging Face…  (ex.: qwen3.5 4b, gemma 3n)",
  "hub.search": "Buscar",
  "hub.hint":
    'Busque um modelo para começar. Dica: repositórios "GGUF" prontos costumam vir de bartowski, unsloth e lmstudio-community. Os downloads vão para a pasta LocalAI/models do seu usuário e aparecem na lista da esquerda.',
  "hub.searching": "Buscando…",
  "hub.nothing": "Nada encontrado.",
  "hub.loadingFiles": "Carregando arquivos…",
  "hub.noFiles": "Sem arquivos .gguf neste repositório.",
  "hub.download": "Baixar",
  "hub.downloading": "Baixando {file}",
  "hub.cancel": "Cancelar",
  "hub.alreadyDownloading": "[hub] já existe um download em andamento",
  "hub.doneLog": "[hub] download concluído: {path}",
  "hub.done": "✓ Baixado: {path}",

  // ----- Tema / idioma -----
  "theme.title": "Tema",
  "theme.light": "Claro",
  "theme.dark": "Escuro",
  "theme.system": "Sistema",
  "theme.nature": "Natureza",
  "theme.darkblue": "Azul escuro",
  "theme.calmgreen": "Verde calmo",
  "theme.pastelpink": "Rosa pastel",
  "theme.punkprincess": "PunkPrincess",
  "lang.title": "Idioma",
} as const;

export type MessageKey = keyof typeof pt;

const en: Record<MessageKey, string> = {
  "brand.sub": "GGUF on CPU/iGPU",
  "sidebar.hwDetecting": "Detecting hardware…",
  "sidebar.folders": "Model folders",
  "sidebar.addFolder": "+ folder",
  "sidebar.models": "GGUF models",
  "sidebar.foldersEmpty": "No folders. Add one.",

  "topbar.noModel": "No model selected",

  "tab.chat": "Chat",
  "tab.tuner": "Settings & Auto-tuner",
  "tab.hub": "Download models",
  "tab.logs": "Logs",

  "status.stopped": "stopped",
  "status.starting": "starting",
  "status.ready": "ready",
  "status.error": "error",
  "status.loadModel": "load a model",

  "btn.load": "Load",
  "btn.starting": "Starting…",
  "btn.stop": "Stop",
  "btn.send": "Send",

  "hw.cores": "Cores",
  "hw.coresVal": "{phys} phys / {log} log",
  "hw.backend": "Backend",
  "hw.threads": "Tuner: {gen} threads for generation, {batch} for prompt",
  "hw.gpu": "GPU (Vulkan): {name} — ~{gb} GB for offload",
  "hw.gpuNone": "Vulkan GPU not detected — estimating ~{gb} GB for offload",

  "models.scanning": "Searching…",
  "models.none": "No .gguf found.",
  "model.vision": "👁 vision",

  "tuner.selectModel": "Select a model.",
  "tuner.controls": "Controls",
  "tuner.ctxTokens": "Context (tokens)",
  "tuner.gpuOffload": "Full GPU offload ({gpu})",
  "tuner.recommended": " — recommended",
  "tuner.gpuLayers": "GPU layers ({n}/{max}) — partial usually hurts",
  "tuner.kvQuant": "Quantized KV cache (q8_0)",
  "tuner.vision": "Vision / multimodal (mmproj) — slower",
  "tuner.speculative": "Speculative decoding (draft: {name})",
  "tuner.port": "Port",
  "tuner.resultConfig": "Resulting configuration",
  "tuner.fitsRam": "fits in RAM",
  "tuner.nearRamLimit": "near the RAM limit",
  "tuner.ramOf": " (of {total} GB)",
  "tuner.cmdLine": "Command line:",
  "tuner.why": "Why these choices (for your hardware)",
  "tuner.warnings": "Warnings",
  "flag.threadsGen": "threads (generation)",
  "flag.threadsPrompt": "threads (prompt)",
  "flag.ctx": "context",

  "common.yes": "yes",
  "common.no": "no",
  "common.on": "on",
  "common.off": "off",

  "log.loading": "\n=== Loading {model} ===",
  "log.portAdjusted": "[localai] port adjusted to {port}",
  "log.errorPrefix": "ERROR: {e}",
  "log.serverReady": "[localai] server signaled ready",
  "err.processEnded": "process ended during loading",
  "err.healthyTimeout": "timeout waiting for the server to become healthy",
  "err.serverResponded": "Server responded {status}: {text}",

  "chat.empty":
    "Load a model and start chatting. The tok/s metrics show up at the top.",
  "chat.newChat": "＋ New chat",
  "chat.sampling": "Sampling & system prompt",
  "samp.temperature": "Temperature",
  "chat.sysPlaceholder": "System prompt (optional) — empty by default",
  "chat.thinkMode":
    "Reasoning mode (think) — off by default (applies instantly, no restart)",
  "chat.attachTitle":
    "Attach image (requires a vision model loaded with mmproj enabled in the tuner)",
  "chat.inputPlaceholder":
    "Write your message…  (Enter sends, Shift+Enter for a new line)",
  "chat.noVisionWarn":
    "⚠ the current model has no vision — reload it with “Vision / multimodal (mmproj)” enabled in the tuner",
  "conv.newChat": "New chat",
  "role.you": "You",
  "role.assistant": "Assistant",
  "chat.copy": "copy",
  "chat.copyTitle": "Copy answer",
  "chat.copied": "✓ copied",
  "chat.thinking": "💭 Thinking…",
  "chat.thoughtOnly":
    "(the model answered only in the thinking channel — open 'Thinking' above)",
  "chat.errorInline": "\n\n[error: {e}]",
  "chat.image": "Image",
  "chat.describeImage": "Describe the image.",

  "ctx.title": "Context tokens used",
  "ctx.titleFull": "Context tokens used (prompt + answer)",
  "ctx.fullWarn":
    "Context almost full: the server will truncate the old messages. Increase the context in the tuner or start a new chat.",

  "hub.searchPlaceholder":
    "Search GGUF models on Hugging Face…  (e.g. qwen3.5 4b, gemma 3n)",
  "hub.search": "Search",
  "hub.hint":
    'Search a model to get started. Tip: ready-made "GGUF" repos usually come from bartowski, unsloth and lmstudio-community. Downloads go to your user\'s LocalAI/models folder and appear in the list on the left.',
  "hub.searching": "Searching…",
  "hub.nothing": "Nothing found.",
  "hub.loadingFiles": "Loading files…",
  "hub.noFiles": "No .gguf files in this repository.",
  "hub.download": "Download",
  "hub.downloading": "Downloading {file}",
  "hub.cancel": "Cancel",
  "hub.alreadyDownloading": "[hub] a download is already in progress",
  "hub.doneLog": "[hub] download finished: {path}",
  "hub.done": "✓ Downloaded: {path}",

  "theme.title": "Theme",
  "theme.light": "Light",
  "theme.dark": "Dark",
  "theme.system": "System",
  "theme.nature": "Nature",
  "theme.darkblue": "Dark blue",
  "theme.calmgreen": "Calm green",
  "theme.pastelpink": "Pastel pink",
  "theme.punkprincess": "PunkPrincess",
  "lang.title": "Language",
};

const es: Record<MessageKey, string> = {
  "brand.sub": "GGUF en CPU/iGPU",
  "sidebar.hwDetecting": "Detectando hardware…",
  "sidebar.folders": "Carpetas de modelos",
  "sidebar.addFolder": "+ carpeta",
  "sidebar.models": "Modelos GGUF",
  "sidebar.foldersEmpty": "Ninguna carpeta. Agrega una.",

  "topbar.noModel": "Ningún modelo seleccionado",

  "tab.chat": "Chat",
  "tab.tuner": "Ajustes & Auto-tuner",
  "tab.hub": "Descargar modelos",
  "tab.logs": "Registros",

  "status.stopped": "detenido",
  "status.starting": "iniciando",
  "status.ready": "listo",
  "status.error": "error",
  "status.loadModel": "carga un modelo",

  "btn.load": "Cargar",
  "btn.starting": "Iniciando…",
  "btn.stop": "Detener",
  "btn.send": "Enviar",

  "hw.cores": "Núcleos",
  "hw.coresVal": "{phys} fís / {log} lóg",
  "hw.backend": "Backend",
  "hw.threads": "Tuner: {gen} threads para generación, {batch} para prompt",
  "hw.gpu": "GPU (Vulkan): {name} — ~{gb} GB para offload",
  "hw.gpuNone": "GPU Vulkan no detectada — estimando ~{gb} GB para offload",

  "models.scanning": "Buscando…",
  "models.none": "No se encontró ningún .gguf.",
  "model.vision": "👁 visión",

  "tuner.selectModel": "Selecciona un modelo.",
  "tuner.controls": "Controles",
  "tuner.ctxTokens": "Contexto (tokens)",
  "tuner.gpuOffload": "Offload total en la GPU ({gpu})",
  "tuner.recommended": " — recomendado",
  "tuner.gpuLayers": "Capas en la GPU ({n}/{max}) — parcial suele empeorar",
  "tuner.kvQuant": "KV cache cuantizado (q8_0)",
  "tuner.vision": "Visión / multimodal (mmproj) — más lento",
  "tuner.speculative": "Decodificación especulativa (borrador: {name})",
  "tuner.port": "Puerto",
  "tuner.resultConfig": "Configuración resultante",
  "tuner.fitsRam": "cabe en la RAM",
  "tuner.nearRamLimit": "cerca del límite de RAM",
  "tuner.ramOf": " (de {total} GB)",
  "tuner.cmdLine": "Línea de comandos:",
  "tuner.why": "Por qué estas elecciones (para tu hardware)",
  "tuner.warnings": "Avisos",
  "flag.threadsGen": "threads (generación)",
  "flag.threadsPrompt": "threads (prompt)",
  "flag.ctx": "contexto",

  "common.yes": "sí",
  "common.no": "no",
  "common.on": "activado",
  "common.off": "desactivado",

  "log.loading": "\n=== Cargando {model} ===",
  "log.portAdjusted": "[localai] puerto ajustado a {port}",
  "log.errorPrefix": "ERROR: {e}",
  "log.serverReady": "[localai] el servidor indicó que está listo",
  "err.processEnded": "el proceso terminó durante la carga",
  "err.healthyTimeout": "tiempo agotado esperando que el servidor esté saludable",
  "err.serverResponded": "El servidor respondió {status}: {text}",

  "chat.empty":
    "Carga un modelo y empieza a conversar. Las métricas de tok/s aparecen arriba.",
  "chat.newChat": "＋ Nuevo chat",
  "chat.sampling": "Muestreo & system prompt",
  "samp.temperature": "Temperatura",
  "chat.sysPlaceholder": "System prompt (opcional) — vacío por defecto",
  "chat.thinkMode":
    "Modo razonamiento (pensar) — desactivado por defecto (se aplica al instante, sin reiniciar)",
  "chat.attachTitle":
    "Adjuntar imagen (requiere un modelo con visión cargado con mmproj activado en el tuner)",
  "chat.inputPlaceholder":
    "Escribe tu mensaje…  (Enter envía, Shift+Enter salta de línea)",
  "chat.noVisionWarn":
    "⚠ el modelo actual no tiene visión — recárgalo con “Visión / multimodal (mmproj)” activado en el tuner",
  "conv.newChat": "Nuevo chat",
  "role.you": "Tú",
  "role.assistant": "Asistente",
  "chat.copy": "copiar",
  "chat.copyTitle": "Copiar respuesta",
  "chat.copied": "✓ copiado",
  "chat.thinking": "💭 Pensando…",
  "chat.thoughtOnly":
    "(el modelo respondió solo en el canal de pensamiento — abre 'Pensando' arriba)",
  "chat.errorInline": "\n\n[error: {e}]",
  "chat.image": "Imagen",
  "chat.describeImage": "Describe la imagen.",

  "ctx.title": "Tokens de contexto usados",
  "ctx.titleFull": "Tokens de contexto usados (prompt + respuesta)",
  "ctx.fullWarn":
    "Contexto casi lleno: el servidor truncará los mensajes antiguos. Aumenta el contexto en el tuner o empieza un chat nuevo.",

  "hub.searchPlaceholder":
    "Buscar modelos GGUF en Hugging Face…  (ej.: qwen3.5 4b, gemma 3n)",
  "hub.search": "Buscar",
  "hub.hint":
    'Busca un modelo para empezar. Consejo: los repositorios "GGUF" listos suelen venir de bartowski, unsloth y lmstudio-community. Las descargas van a la carpeta LocalAI/models de tu usuario y aparecen en la lista de la izquierda.',
  "hub.searching": "Buscando…",
  "hub.nothing": "No se encontró nada.",
  "hub.loadingFiles": "Cargando archivos…",
  "hub.noFiles": "No hay archivos .gguf en este repositorio.",
  "hub.download": "Descargar",
  "hub.downloading": "Descargando {file}",
  "hub.cancel": "Cancelar",
  "hub.alreadyDownloading": "[hub] ya hay una descarga en curso",
  "hub.doneLog": "[hub] descarga finalizada: {path}",
  "hub.done": "✓ Descargado: {path}",

  "theme.title": "Tema",
  "theme.light": "Claro",
  "theme.dark": "Oscuro",
  "theme.system": "Sistema",
  "theme.nature": "Naturaleza",
  "theme.darkblue": "Azul oscuro",
  "theme.calmgreen": "Verde tranquilo",
  "theme.pastelpink": "Rosa pastel",
  "theme.punkprincess": "PunkPrincess",
  "lang.title": "Idioma",
};

const DICTS: Record<Locale, Record<MessageKey, string>> = { pt, en, es };

/** Palpite de locale pelo idioma do sistema (só no 1º uso). */
export function detectLocale(): Locale {
  const l = (
    typeof navigator !== "undefined" ? navigator.language : "pt"
  ).toLowerCase();
  if (l.startsWith("en")) return "en";
  if (l.startsWith("es")) return "es";
  return "pt";
}

export function getLocale(): Locale {
  const v =
    typeof localStorage !== "undefined"
      ? localStorage.getItem(LOCALE_KEY)
      : null;
  return v === "pt" || v === "en" || v === "es" ? v : detectLocale();
}

/**
 * Persiste o idioma e recarrega a página. O reload é a "reatividade": como o
 * app monta tudo no boot, recarregar re-renderiza com o dicionário novo.
 */
export function setLocale(locale: Locale) {
  if (locale === getLocale()) return;
  try {
    localStorage.setItem(LOCALE_KEY, locale);
  } catch {
    /* localStorage indisponível */
  }
  location.reload();
}

/** Traduz uma chave, interpolando placeholders `{param}`. */
export function t(
  key: MessageKey,
  params?: Record<string, string | number>,
): string {
  let msg: string = DICTS[getLocale()][key] ?? pt[key] ?? key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      msg = msg.split(`{${k}}`).join(String(v));
    }
  }
  return msg;
}

/** Tag BCP-47 do locale atual (p/ formatação de números/datas e <html lang>). */
export function localeTag(): string {
  const map: Record<Locale, string> = {
    pt: "pt-BR",
    en: "en-US",
    es: "es-ES",
  };
  return map[getLocale()];
}
