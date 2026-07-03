// Cliente de chat para o llama-server (API compativel com OpenAI),
// com streaming SSE e captura das metricas (tok/s) do llama.cpp.

/** Parte de conteudo multimodal (texto ou imagem em data-URL base64). */
export type ContentPart =
  | { type: "text"; text: string }
  | { type: "image_url"; image_url: { url: string } };

export interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string | ContentPart[];
  /** Pensamento capturado do modelo (so para UI/persistencia; nunca reenviado a API). */
  reasoning?: string;
}

export interface SamplingParams {
  temperature: number;
  top_p: number;
  top_k: number;
  min_p: number;
  repeat_penalty: number;
  max_tokens: number;
}

export interface Timings {
  prompt_n?: number;
  prompt_per_second?: number;
  predicted_n?: number;
  predicted_per_second?: number;
}

export interface Usage {
  prompt_tokens?: number;
  completion_tokens?: number;
  total_tokens?: number;
}

export interface StreamChunk {
  delta?: string;
  /** canal de "pensamento" de modelos de reasoning (ex.: Qwen3.5) */
  reasoning?: string;
  timings?: Timings;
  /** contagem de tokens do request (prompt completo + gerados) */
  usage?: Usage;
  /** aviso de diagnostico para a aba Logs (ex.: fallback de <think> ativado) */
  note?: string;
  done?: boolean;
}

/** Extrai apenas o texto de um content (string ou parts). */
export function contentText(c: string | ContentPart[]): string {
  if (typeof c === "string") return c;
  return c
    .filter((p): p is Extract<ContentPart, { type: "text" }> => p.type === "text")
    .map((p) => p.text)
    .join("\n");
}

// ---------------------------------------------------------------------------
// Fallback de no-think: alguns GGUFs (finetunes de Gemma/Qwen com chat
// template proprio) IGNORAM reasoning_budget/enable_thinking e despejam
// "<think> ... </think>" cru no content. Este filtro detecta a tag no comeco
// do stream e redireciona o miolo para o canal de reasoning, para a resposta
// nao comecar com o monologo interno. Tags podem chegar partidas entre chunks.
// ---------------------------------------------------------------------------
const THINK_TAGS: Array<{ open: string; close: string }> = [
  { open: "<think>", close: "</think>" },
  { open: "<thinking>", close: "</thinking>" },
];

class ThinkFilter {
  private mode: "detect" | "reasoning" | "answer" = "detect";
  private buf = "";
  private close = "";
  private notified = false;
  private answerStarted = false;

  /** Processa um delta de content; devolve o que vai para cada canal. */
  feed(delta: string): { delta?: string; reasoning?: string; note?: string } {
    if (this.mode === "answer") return { delta: this.pass(delta) };
    this.buf += delta;

    if (this.mode === "detect") {
      const lead = this.buf.length - this.buf.trimStart().length;
      const body = this.buf.slice(lead);
      const hit = THINK_TAGS.find((t) => body.startsWith(t.open));
      if (hit) {
        this.mode = "reasoning";
        this.close = hit.close;
        this.buf = body.slice(hit.open.length);
        const note = this.notified
          ? undefined
          : `[no-think] modelo emitiu ${hit.open} cru no texto (template ignorou reasoning_budget); redirecionando para o canal de pensamento`;
        this.notified = true;
        const out = this.drainReasoning();
        return { ...out, note };
      }
      // ainda pode ser prefixo de uma tag chegando em pedacos? segura o buffer
      if (body.length === 0 || THINK_TAGS.some((t) => t.open.startsWith(body))) {
        return {};
      }
      // nao e tag de pensamento: vira resposta normal
      this.mode = "answer";
      const out = this.buf;
      this.buf = "";
      return { delta: this.pass(out) };
    }

    // mode === "reasoning"
    return this.drainReasoning();
  }

  /** No fim do stream, esvazia o que sobrou no buffer. */
  flush(): { delta?: string; reasoning?: string } {
    const rest = this.buf;
    this.buf = "";
    if (!rest) return {};
    if (this.mode === "reasoning") return { reasoning: rest };
    return { delta: this.pass(rest) };
  }

  private drainReasoning(): { delta?: string; reasoning?: string } {
    const idx = this.buf.indexOf(this.close);
    if (idx >= 0) {
      const reasoning = this.buf.slice(0, idx);
      const after = this.buf.slice(idx + this.close.length);
      this.mode = "answer";
      this.buf = "";
      return {
        reasoning: reasoning || undefined,
        delta: after ? this.pass(after) : undefined,
      };
    }
    // segura uma cauda que possa ser o comeco de "</think>" partido
    const keep = this.partialTail();
    const reasoning = this.buf.slice(0, this.buf.length - keep);
    this.buf = this.buf.slice(this.buf.length - keep);
    return { reasoning: reasoning || undefined };
  }

  private partialTail(): number {
    const max = Math.min(this.close.length - 1, this.buf.length);
    for (let n = max; n > 0; n--) {
      if (this.buf.endsWith(this.close.slice(0, n))) return n;
    }
    return 0;
  }

  /** Remove whitespace inicial da resposta (sobra tipica apos </think>). */
  private pass(s: string): string {
    if (this.answerStarted) return s;
    const t = s.replace(/^\s+/, "");
    if (t) this.answerStarted = true;
    return t;
  }
}

export async function* streamChat(
  port: number,
  messages: ChatMessage[],
  params: SamplingParams,
  signal: AbortSignal,
  think: boolean = false,
): AsyncGenerator<StreamChunk> {
  const body = {
    model: "local",
    // reasoning capturado e so para UI/persistencia — nao vai para a API
    messages: messages.map(({ role, content }) => ({ role, content })),
    stream: true,
    stream_options: { include_usage: true },
    cache_prompt: true,
    temperature: params.temperature,
    top_p: params.top_p,
    top_k: params.top_k,
    min_p: params.min_p,
    repeat_penalty: params.repeat_penalty,
    max_tokens: params.max_tokens,
    // Controle de reasoning por request (llama-server):
    //   reasoning_budget = 0  -> encerra o pensamento na hora (resposta direta)
    //   reasoning_budget = -1 -> pensamento livre
    // Funciona nos Qwen3 / 3.5 / 3.6 e familias hibridas SEM reiniciar o
    // servidor. O enable_thinking via template e reforco para modelos cujo
    // chat template decide pelo kwarg. Para templates que ignoram ambos,
    // o ThinkFilter acima captura o <think> cru no stream (fallback).
    reasoning_budget: think ? -1 : 0,
    chat_template_kwargs: { enable_thinking: think },
  };

  const resp = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });

  if (!resp.ok || !resp.body) {
    const txt = await resp.text().catch(() => "");
    throw new Error(`Servidor respondeu ${resp.status}: ${txt}`);
  }

  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  const filter = new ThinkFilter();

  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });

    const lines = buffer.split("\n");
    buffer = lines.pop() ?? "";

    for (const raw of lines) {
      const line = raw.trim();
      if (!line.startsWith("data:")) continue;
      const data = line.slice(5).trim();
      if (data === "[DONE]") {
        const rest = filter.flush();
        if (rest.delta || rest.reasoning) yield rest;
        yield { done: true };
        return;
      }
      try {
        const obj = JSON.parse(data);
        const d = obj?.choices?.[0]?.delta;
        const rawDelta: string | undefined = d?.content;
        // llama.cpp expoe o pensamento em reasoning_content (ou variantes)
        const reasoning: string | undefined =
          d?.reasoning_content ?? d?.reasoning;
        const timings: Timings | undefined = obj?.timings;
        const usage: Usage | undefined = obj?.usage ?? undefined;

        let delta: string | undefined;
        let fbReasoning: string | undefined;
        let note: string | undefined;
        if (rawDelta) {
          const f = filter.feed(rawDelta);
          delta = f.delta || undefined;
          fbReasoning = f.reasoning;
          note = f.note;
        }
        const outReasoning =
          reasoning || fbReasoning
            ? `${reasoning ?? ""}${fbReasoning ?? ""}`
            : undefined;
        if (delta || outReasoning || timings || usage || note) {
          yield { delta, reasoning: outReasoning, timings, usage, note };
        }
      } catch {
        // chunk parcial/ruido — ignora
      }
    }
  }
  const rest = filter.flush();
  if (rest.delta || rest.reasoning) yield rest;
  yield { done: true };
}

/// Verifica /health e retorna o id do modelo carregado, se disponivel.
export async function fetchModelId(port: number): Promise<string | null> {
  try {
    const r = await fetch(`http://127.0.0.1:${port}/v1/models`);
    if (!r.ok) return null;
    const j = await r.json();
    return j?.data?.[0]?.id ?? null;
  } catch {
    return null;
  }
}
