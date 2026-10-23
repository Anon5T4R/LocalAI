import { describe, expect, it } from "vitest";
import {
  buildDocInstruction,
  detectDocRequest,
  docFilenameFromMd,
  extractDocMarkdown,
} from "../../docmode";

describe("detectDocRequest", () => {
  it("pedidos claros de arquivo ativam, com o formato certo", () => {
    expect(detectDocRequest("Cria um documento .docx com uma lista de países")?.format).toBe("docx");
    expect(detectDocRequest("Crie um documento word sobre gatos")?.format).toBe("docx");
    expect(detectDocRequest("Faça uma página html com o conteúdo X")?.format).toBe("html");
    expect(detectDocRequest("Cria uma página sobre gatos")?.format).toBe("html");
    expect(detectDocRequest("Salve um markdown com a receita")?.format).toBe("md");
    expect(detectDocRequest("gere um arquivo receita.txt")?.format).toBe("txt");
    expect(detectDocRequest("write a document about dogs")?.format).toBe("docx");
    expect(detectDocRequest("crie um arquivo de texto simples")?.format).toBe("docx");
  });

  it("pedidos SEM arquivo não ativam (o modelo responde no chat)", () => {
    expect(detectDocRequest("Crie uma lista de países para visitar")).toBeNull();
    expect(detectDocRequest("Escreva um poema sobre o mar")).toBeNull();
    expect(detectDocRequest("Qual a capital da França?")).toBeNull();
    expect(detectDocRequest("faz um resumo do documento anexado")).toBeNull();
  });

  it("menção a arquivo sem ação não ativa", () => {
    expect(detectDocRequest("O que é um arquivo .docx?")).toBeNull();
  });
});

describe("extractDocMarkdown", () => {
  it("prefere o fence ```markdown quando existe", () => {
    const out = extractDocMarkdown("Aqui está:\n```markdown\n# Título\ncorpo\n```\nqualquer coisa");
    expect(out).toBe("# Título\ncorpo");
  });

  it("sem fence, remove aberturas de conversa e mantém o documento", () => {
    const out = extractDocMarkdown("Aqui está o documento solicitado:\n\n# Lista\n\n- a\n- b");
    expect(out).toBe("# Lista\n\n- a\n- b");
  });

  it("texto que já é o documento passa intacto", () => {
    expect(extractDocMarkdown("# Título\ncorpo")).toBe("# Título\ncorpo");
  });
});

describe("docFilenameFromMd", () => {
  it("usa o primeiro heading como nome", () => {
    expect(docFilenameFromMd("# Lista de Países\n\ncorpo")).toBe("Lista de Países");
  });

  it("sem heading, nome com data/hora (nunca vazio)", () => {
    const n = docFilenameFromMd("sem heading aqui");
    expect(n).toMatch(/^documento-\d{8}-\d{6}$/);
  });
});

describe("buildDocInstruction", () => {
  it("menciona o formato e pede só o conteúdo", () => {
    const i = buildDocInstruction("docx");
    expect(i).toContain(".docx");
    expect(i).toContain("MARKDOWN");
    expect(i).toContain("SOMENTE");
  });
});
