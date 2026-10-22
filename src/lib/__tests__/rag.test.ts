import { describe, expect, it } from "vitest";
import {
  buildRagContext,
  decideStrategy,
  estimateTokens,
  formatSearchReply,
  type RagChunk,
  type RagSource,
} from "../../rag";

describe("estimateTokens", () => {
  it("estima chars/4 arredondando pra cima", () => {
    expect(estimateTokens(0)).toBe(0);
    expect(estimateTokens(1)).toBe(1);
    expect(estimateTokens(8000)).toBe(2000);
    expect(estimateTokens(8001)).toBe(2001);
  });
});

describe("decideStrategy", () => {
  it("documento pequeno cabe: modo full, sem retrieval", () => {
    // 8192 ctx × 0.55 = 4505 tokens de orçamento; 12000 chars = 3000 tokens
    const s = decideStrategy(12000, 8192);
    expect(s.mode).toBe("full");
    expect(s.k).toBe(0);
  });

  it("documento grande: modo search com k adaptado ao orçamento", () => {
    // 8192 × 0.55 = 4505 tokens / 550 por chunk = 8
    const s = decideStrategy(200000, 8192);
    expect(s.mode).toBe("search");
    expect(s.k).toBe(8);
  });

  it("k respeita o piso de 3 mesmo em contexto apertado", () => {
    const s = decideStrategy(200000, 2048);
    expect(s.mode).toBe("search");
    expect(s.k).toBe(3);
  });

  it("k respeita o teto de 10 em contexto largo", () => {
    const s = decideStrategy(2000000, 131072);
    expect(s.mode).toBe("search");
    expect(s.k).toBe(10);
  });

  it("na fronteira exata do orçamento ainda cabe (<=)", () => {
    // 8192 × 0.55 = 4505 tokens; 4505 × 4 = 18020 chars
    const s = decideStrategy(18020, 8192);
    expect(s.mode).toBe("full");
  });
});

describe("buildRagContext", () => {
  const chunks: RagSource[] = [
    { name: "relatorio.pdf", page: 3, text: "trecho da página três", full: false },
    { name: "relatorio.pdf", page: 7, text: "trecho da página sete", full: false },
  ];

  it("sem fontes devolve a pergunta pura", () => {
    expect(buildRagContext([], "qual a pergunta?")).toBe("qual a pergunta?");
  });

  it("trechos citados com doc e página, régua e pergunta no fim", () => {
    const out = buildRagContext(chunks, "o que diz o documento?");
    expect(out).toBe(
      "[relatorio.pdf, p. 3]\ntrecho da página três\n\n" +
        "[relatorio.pdf, p. 7]\ntrecho da página sete\n\n" +
        "---\n\no que diz o documento?",
    );
  });

  it("documento inteiro ganha rótulo próprio (sem página)", () => {
    const full: RagSource[] = [
      { name: "artigo.pdf", page: null, text: "texto inteiro aqui", full: true },
    ];
    const out = buildRagContext(full, "resuma");
    expect(out).toContain("[artigo.pdf — documento completo]\ntexto inteiro aqui");
    expect(out).toContain("---\n\nresuma");
  });

  it("documento sem paginação (docx/txt/md) cita só o nome", () => {
    const flat: RagSource[] = [
      { name: "ata.docx", page: 0, text: "trecho da ata", full: false },
    ];
    const out = buildRagContext(flat, "o que ficou decidido?");
    expect(out).toContain("[ata.docx]\ntrecho da ata");
    expect(out).not.toContain("p. 0");
  });

  it("NÃO injeta instrução nenhuma (só conteúdo + pergunta)", () => {
    const out = buildRagContext(chunks, "pergunta");
    // nenhuma frase imperativa típica de prompt engineering oculto
    expect(out).not.toMatch(/responda|baseie|use apenas|ignore|você deve/i);
  });
});

describe("formatSearchReply", () => {
  const chunk = (page: number, text: string): RagChunk => ({
    doc_id: "d1",
    doc_name: "manual.pdf",
    page,
    seq: page,
    text,
  });

  it("sem trechos devolve aviso curto (o modelo entende que não achou)", () => {
    expect(formatSearchReply([])).toMatch(/no excerpts/i);
  });

  it("cita doc e página no mesmo formato do contexto injetado", () => {
    const out = formatSearchReply([chunk(2, "alfa"), chunk(9, "beta")]);
    expect(out).toBe("[manual.pdf, p. 2]\nalfa\n\n[manual.pdf, p. 9]\nbeta");
  });

  it("documento sem paginação cita só o nome (page 0)", () => {
    const out = formatSearchReply([chunk(0, "docx sem páginas")]);
    expect(out).toBe("[manual.pdf]\ndocx sem páginas");
    expect(out).not.toContain("p. 0");
  });
});
