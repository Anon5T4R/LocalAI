// Modo documento: o APP identifica pedidos de criação de arquivo e orquestra —
// pede só o CONTEÚDO ao modelo (que modelos pequenos fazem bem) e salva com a
// mesma ferramenta `save_document`. Sem tool calling nativo, sem loop, menos
// tokens. Proposta do João (2026-10-23): "o app identifica pedidos que
// deveriam ser tool calling e cria pelo modelo — economiza computação".
//
// Transparência: a resposta vira o conteúdo do documento na conversa + o chip
// "🛠 save_document → arquivo" (mesmo mecanismo das tools). Nada oculto.

export type DocFormat = "docx" | "html" | "md" | "txt";

/** Ações de criação/salvamento em PT/EN/ES. */
const ACTION =
  /\b(crie|cria|criar|fa[çc]a|faz|fazer|gere|gerar|monte|montar|escreva|escrever|salve|salva|salvar|make|create|write|generate|save)\b/i;

/** Alvos que caracterizam ARQUIVO (não basta "lista"/"texto" no chat). */
const FILE =
  /(\bdocumentos?\b|\barquivos?\b|\bp[áa]ginas?\b|\bdocx\b|\bword\b|\bhtml\b|\bmarkdown\b|\.md\b|\.docx\b|\.txt\b|\.html\b|\bdocuments?\b|\bfiles?\b|\bpages?\b)/i;

/**
 * Detecta se a mensagem do usuário é um pedido de CRIAÇÃO de documento.
 * Exige ação (criar/salvar/…) + alvo de arquivo na mesma frase — "crie uma
 * lista de países" (sem arquivo) NÃO ativa. `null` = não é pedido de arquivo.
 */
export function detectDocRequest(text: string): { format: DocFormat } | null {
  if (!ACTION.test(text) || !FILE.test(text)) return null;
  const t = text.toLowerCase();
  // "faz um resumo do documento anexado": o documento é OBJETO DE LEITURA,
  // não saída — só ativa se houver formato de saída explícito
  if (/\b(anexad[oa]s?|anexos?|anexo)\b/.test(t) && !/\b(docx|word|html|\.md|\.txt)\b/.test(t))
    return null;
  if (/\bhtml\b|\bp[áa]ginas?\b/.test(t)) return { format: "html" };
  if (/\.md\b|\bmarkdown\b/.test(t)) return { format: "md" };
  if (/\.txt\b/.test(t)) return { format: "txt" };
  if (/\bdocx\b|\bword\b|\bdocumentos?\b|\barquivos?\b/.test(t))
    return { format: "docx" };
  return { format: "docx" };
}

/** Instrução que acompanha a pergunta (visível no histórico da conversa). */
export function buildDocInstruction(format: DocFormat): string {
  return (
    `(O app salvará esta resposta como arquivo .${format}.) ` +
    "Escreva o conteúdo completo do documento em MARKDOWN, começando com um " +
    "título (#). Responda SOMENTE com o conteúdo do documento — sem " +
    "introdução, sem comentários e sem cercas de código em volta."
  );
}

/** Limpa a resposta do modelo: extrai o markdown do documento (fence
 *  ```markdown se houver; senão o texto, sem frases de abertura). */
export function extractDocMarkdown(text: string): string {
  const fence = text.match(/```(?:markdown|md)\s*\n([\s\S]*?)```/i);
  if (fence) return fence[1].trim();
  let out = text.trim();
  // remove aberturas de conversa antes do documento ("Aqui está…", "Segue…")
  out = out.replace(
    /^(aqui (est[áa]|vai)|segue|claro|certamente|com prazer)[^\n#]*\n+/i,
    "",
  );
  return out.trim();
}

/** Nome de arquivo a partir do primeiro heading do markdown (ou data/hora). */
export function docFilenameFromMd(md: string): string {
  const h = md.match(/^#\s+(.{1,80})$/m);
  if (h && h[1].trim()) return h[1].trim();
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `documento-${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(
    d.getHours(),
  )}${p(d.getMinutes())}${p(d.getSeconds())}`;
}
