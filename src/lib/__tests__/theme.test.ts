import { afterEach, describe, expect, it, vi } from "vitest";
import { THEME_PREFS, applyTheme, getThemePref } from "../theme";

/** Stub minimo de localStorage (o ambiente de teste e node, nao tem DOM). */
function fakeStorage(value: string | null) {
  vi.stubGlobal("localStorage", {
    getItem: () => value,
    setItem: () => {},
  });
}

/** Stub do <html> pra ler o data-theme que o applyTheme escreve. */
function fakeDocument() {
  const dataset: Record<string, string> = {};
  vi.stubGlobal("document", { documentElement: { dataset } });
  return dataset;
}

afterEach(() => vi.unstubAllGlobals());

describe("getThemePref", () => {
  // O valor do localStorage vem de versoes antigas do app e de edicao manual.
  // Sem a validacao ele iria cru pro data-theme e o CSS nao casaria com regra
  // nenhuma — a tela fica sem paleta, nao no tema padrao.
  it("valor desconhecido no storage cai em system", () => {
    fakeStorage("escuro");
    expect(getThemePref()).toBe("system");
  });

  // Lista escrita a mao de proposito: e o espelho dos seletores
  // :root[data-theme="..."] do styles.css (mais o "dark", que e o :root, e o
  // "system"). Se um tema for adicionado ao CSS/ao seletor e esquecido em
  // THEME_PREFS, a validacao do getThemePref o rejeita em silencio e a escolha
  // do usuario volta pra system no proximo boot — este teste quebra antes.
  it("aceita todos os temas que o CSS define", () => {
    const doCss = [
      "system",
      "light",
      "dark",
      "nature",
      "darkblue",
      "calmgreen",
      "pastelpink",
      "punkprincess",
    ];
    expect([...THEME_PREFS].sort()).toEqual([...doCss].sort());
    for (const pref of doCss) {
      fakeStorage(pref);
      expect(getThemePref()).toBe(pref);
    }
  });
});

describe("applyTheme", () => {
  // "system" nao existe como paleta no CSS: se fosse escrito cru no data-theme
  // nenhuma regra casaria. Tem que ser resolvido pra dark/light antes.
  it("resolve system pelo prefers-color-scheme", () => {
    const dataset = fakeDocument();
    vi.stubGlobal("matchMedia", () => ({ matches: true }));
    applyTheme("system");
    expect(dataset.theme).toBe("dark");

    vi.stubGlobal("matchMedia", () => ({ matches: false }));
    applyTheme("system");
    expect(dataset.theme).toBe("light");
  });

  // O caminho oposto: os temas nomeados sao paletas fixas e NAO podem ser
  // reduzidos a dark/light — perderiam o accent proprio.
  it("tema nomeado vai direto pro data-theme, sem resolucao", () => {
    const dataset = fakeDocument();
    vi.stubGlobal("matchMedia", () => ({ matches: true }));
    applyTheme("punkprincess");
    expect(dataset.theme).toBe("punkprincess");
  });
});
