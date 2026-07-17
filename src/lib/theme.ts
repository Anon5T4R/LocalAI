// Tema do LocalAI Studio (vanilla, sem React).
// O dark é o default histórico (o `:root` do styles.css). O light entra via
// `:root[data-theme="light"]`. "system" segue o prefers-color-scheme. Os temas
// NOMEADOS são paletas fixas (`:root[data-theme="<nome>"]`) que sobrepõem
// inclusive o accent — vão direto pro data-theme, sem resolução.

export type ThemePref =
  | "system"
  | "light"
  | "dark"
  | "nature"
  | "darkblue"
  | "calmgreen"
  | "pastelpink"
  | "punkprincess";

/** Ordem canônica (usada pelo seletor das configurações). */
export const THEME_PREFS: ThemePref[] = [
  "system",
  "light",
  "dark",
  "nature",
  "darkblue",
  "calmgreen",
  "pastelpink",
  "punkprincess",
];

const THEME_KEY = "localai.theme";

export function getThemePref(): ThemePref {
  const v =
    typeof localStorage !== "undefined"
      ? localStorage.getItem(THEME_KEY)
      : null;
  return THEME_PREFS.includes(v as ThemePref) ? (v as ThemePref) : "system";
}

function systemIsDark(): boolean {
  return (
    typeof matchMedia !== "undefined" &&
    matchMedia("(prefers-color-scheme: dark)").matches
  );
}

/** Resolve a preferência para o tema efetivo e aplica no <html>. */
export function applyTheme(pref: ThemePref = getThemePref()) {
  if (pref === "system") {
    document.documentElement.dataset.theme = systemIsDark() ? "dark" : "light";
    return;
  }
  document.documentElement.dataset.theme = pref;
}

export function setThemePref(pref: ThemePref) {
  try {
    localStorage.setItem(THEME_KEY, pref);
  } catch {
    /* localStorage indisponível */
  }
  applyTheme(pref);
}

/** Aplica no boot e reage a mudanças do sistema quando a pref é "system". */
export function initTheme() {
  applyTheme();
  if (typeof matchMedia !== "undefined") {
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
      if (getThemePref() === "system") applyTheme("system");
    });
  }
}
