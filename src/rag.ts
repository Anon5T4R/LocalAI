// Helpers puros do RAG (sem DOM — testáveis no vitest). A UI e o send()
// vivem no main.ts; as DECISÕES (cabe no contexto? quantos chunks?) e a
// montagem do bloco de contexto vivem aqui.
//
// Princípio combinado com o João: nenhuma instrução oculta. O bloco abaixo
// é só conteúdo + a pergunta, e é exatamente o que o painel "Fontes"
// mostra na UI.

/** Documento anexado à conversa (persistido no conversations.json). */
export interface ConvDoc {
  doc_id: string;
  name: string;
  pages: number;
  chars: number;
}

/** Chunk vindo da busca BM25 (backend). */
export interface RagChunk {
  doc_id: string;
  doc_name: string;
  page: number;
  seq: number;
  text: string;
}

/** O que foi injetado no prompt (mostrado no painel "Fontes"). */
export interface RagSource {
  name: string;
  /** null = documento inteiro anexado (não um trecho de página) */
  page: number | null;
  text: string;
  full: boolean;
}

/** Payload persistido na mensagem do usuário (só UI/persistência — a API
 *  recebe o `context` já montado; nunca o objeto cru). */
export interface RagPayload {
  context: string;
  sources: RagSource[];
}

// Estimativa conservadora: PT/EN/ES ficam em ~3.5-4.5 chars/token; usar 4
// e um orçamento de 55% do ctx deixa folga pro histórico + resposta.
export const CHARS_PER_TOKEN = 4;
export const CTX_BUDGET = 0.55;
/** Custo estimado por chunk injetado (~2000 chars + cabeçalho). */
export const CHUNK_TOKENS = 550;
export const MIN_K = 3;
export const MAX_K = 10;

export function estimateTokens(chars: number): number {
  return Math.ceil(chars / CHARS_PER_TOKEN);
}

/**
 * PDF pequeno que cabe no orçamento de contexto → texto INTEIRO (zero
 * perda, zero retrieval). Senão → busca top-k com k adaptado ao orçamento.
 */
export function decideStrategy(
  totalChars: number,
  ctxSize: number,
): { mode: "full" | "search"; k: number } {
  const budget = Math.floor(ctxSize * CTX_BUDGET);
  if (estimateTokens(totalChars) <= budget) return { mode: "full", k: 0 };
  const k = Math.max(MIN_K, Math.min(MAX_K, Math.floor(budget / CHUNK_TOKENS)));
  return { mode: "search", k };
}

/**
 * Monta o bloco que vai ao modelo: trechos citados (doc, página) + a
 * pergunta, separados por régua. SEM instrução nenhuma — o formato
 * "[doc, p. N]" induz citação naturalmente e tudo é visível na UI.
 */
export function buildRagContext(sources: RagSource[], question: string): string {
  if (!sources.length) return question;
  const blocks = sources.map((s) =>
    s.full
      ? `[${s.name} — documento completo]\n${s.text}`
      : s.page && s.page > 0
        ? `[${s.name}, p. ${s.page}]\n${s.text}`
        : `[${s.name}]\n${s.text}`,
  );
  return `${blocks.join("\n\n")}\n\n---\n\n${question}`;
}

/**
 * Resposta da ferramenta `search_docs` (o que o MODELO lê). Mesmo formato
 * dos trechos injetados — coerente e citável.
 */
export function formatSearchReply(chunks: RagChunk[]): string {
  if (!chunks.length) return "No excerpts found for that query.";
  return chunks
    .map((c) =>
      c.page > 0
        ? `[${c.doc_name}, p. ${c.page}]\n${c.text}`
        : `[${c.doc_name}]\n${c.text}`,
    )
    .join("\n\n");
}
