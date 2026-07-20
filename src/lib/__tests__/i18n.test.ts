import { afterEach, describe, expect, it, vi } from "vitest";
import { detectLocale, getLocale, t } from "../i18n";

function env(language: string, stored: string | null = null) {
  vi.stubGlobal("navigator", { language });
  vi.stubGlobal("localStorage", { getItem: () => stored, setItem: () => {} });
}

afterEach(() => vi.unstubAllGlobals());

describe("detectLocale", () => {
  // navigator.language quase nunca e a tag curta: comparar com "en"/"es" na
  // igualdade manda todo mundo pro portugues.
  it("casa pelo prefixo, nao pela tag inteira", () => {
    env("en-GB");
    expect(detectLocale()).toBe("en");
    env("es-419");
    expect(detectLocale()).toBe("es");
    env("pt-PT");
    expect(detectLocale()).toBe("pt");
  });

  // A tag pode vir maiuscula dependendo do host; sem o toLowerCase o usuario
  // ingles abriria o app em portugues.
  it("nao depende da caixa da tag", () => {
    env("EN-US");
    expect(detectLocale()).toBe("en");
  });

  // Idioma sem traducao cai no pt (fonte da verdade das chaves), nao em branco.
  it("idioma sem dicionario cai no portugues", () => {
    env("de-DE");
    expect(detectLocale()).toBe("pt");
  });
});

describe("getLocale", () => {
  // Um valor invalido no storage (versao antiga, edicao manual) nao pode virar
  // indice do dicionario — daria undefined em toda chave. E o fallback e o
  // palpite do sistema, nao um "pt" fixo.
  it("valor invalido no storage volta pro palpite do sistema", () => {
    env("es-ES", "fr");
    expect(getLocale()).toBe("es");
  });

  // O escolhido explicitamente ganha do idioma do sistema.
  it("a escolha salva prevalece sobre o idioma do sistema", () => {
    env("es-ES", "en");
    expect(getLocale()).toBe("en");
  });
});

describe("t", () => {
  // Interpolacao com numero: sem o String(v) o template ficaria com "[object
  // Object]"/NaN em mensagens de erro e status HTTP.
  it("interpola numero como texto", () => {
    env("pt-BR");
    expect(t("err.serverResponded", { status: 500, text: "boom" })).toBe(
      "Servidor respondeu 500: boom",
    );
  });

  // Faltando um parametro, o placeholder sobra visivel em vez de quebrar a
  // string inteira — bug que aparece so na traducao que tem a chave a mais.
  it("placeholder sem parametro fica literal em vez de sumir", () => {
    env("pt-BR");
    expect(t("err.serverResponded", { status: 500 })).toBe(
      "Servidor respondeu 500: {text}",
    );
  });
});
