/**
 * Theme DOM application (P31 · A3).
 *
 * Built-in dark/light uses the `.dark` / `.light` class on <html> (P25 index.css).
 * A custom theme package is applied by injecting its CSS *variable overrides*
 * into a dedicated <style> tag and flipping `document.documentElement
 * [data-theme]` — exactly the contract the Rust side emits via `theme_changed`.
 */

import { useAppStore } from "@/stores/useAppStore";

const STYLE_ID = "caspian-theme-override";

/** Inject (or replace) the custom-theme override <style>. */
export function injectThemeCss(css: string): void {
  let el = document.getElementById(STYLE_ID) as HTMLStyleElement | null;
  if (!el) {
    el = document.createElement("style");
    el.id = STYLE_ID;
    document.head.appendChild(el);
  }
  el.textContent = css;
}

/** Remove any custom-theme override <style>. */
export function clearThemeCss(): void {
  document.getElementById(STYLE_ID)?.remove();
}

/**
 * Resolve + apply the active theme to <html>.
 *
 * - Custom theme active → inject its CSS overrides, set `data-theme=<name>`,
 *   drop the built-in `.dark`/`.light` classes (the package defines its own
 *   tokens).
 * - Otherwise → built-in dark/light via `.dark`/`.light`, drop the override.
 */
export function applyThemeDom(): void {
  const root = document.documentElement;
  const { theme, customTheme, customThemeCss } = useAppStore.getState();

  if (customTheme && customThemeCss) {
    injectThemeCss(customThemeCss);
    root.setAttribute("data-theme", customTheme);
    root.classList.remove("dark", "light");
    return;
  }

  // Built-in dark/light.
  clearThemeCss();
  root.removeAttribute("data-theme");
  root.classList.toggle("dark", theme === "dark");
  root.classList.toggle("light", theme === "light");
}
