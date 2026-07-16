// Tema claro/escuro/sistema do LocalAI Studio (vanilla, sem React).
// O dark é o default histórico (o `:root` do styles.css). O light entra via
// `:root[data-theme="light"]`. "system" segue o prefers-color-scheme.

export type ThemePref = "system" | "light" | "dark";

const THEME_KEY = "localai.theme";

export function getThemePref(): ThemePref {
  const v =
    typeof localStorage !== "undefined"
      ? localStorage.getItem(THEME_KEY)
      : null;
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

function systemIsDark(): boolean {
  return (
    typeof matchMedia !== "undefined" &&
    matchMedia("(prefers-color-scheme: dark)").matches
  );
}

/** Resolve a preferência para o tema efetivo e aplica no <html>. */
export function applyTheme(pref: ThemePref = getThemePref()) {
  const dark = pref === "dark" || (pref === "system" && systemIsDark());
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

export function setThemePref(pref: ThemePref) {
  try {
    localStorage.setItem(THEME_KEY, pref);
  } catch {
    /* localStorage indisponível */
  }
  applyTheme(pref);
}

/** Cicla system → light → dark → system. */
export function cycleTheme(): ThemePref {
  const order: ThemePref[] = ["system", "light", "dark"];
  const next = order[(order.indexOf(getThemePref()) + 1) % order.length];
  setThemePref(next);
  return next;
}

/** Ícone da preferência atual (☀/🌙/🖥). */
export function themeIcon(pref: ThemePref = getThemePref()): string {
  return pref === "light" ? "☀" : pref === "dark" ? "🌙" : "🖥";
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
