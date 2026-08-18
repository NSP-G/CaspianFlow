import { create } from "zustand";

export type Theme = "dark" | "light";

const STORAGE_KEY = "caspian.theme";
const CUSTOM_KEY = "caspian.customTheme";
const ONBOARDING_KEY = "caspian.onboardingSeen";

interface AppState {
  theme: Theme;
  sidebarCollapsed: boolean;
  /** Active custom theme package name (P31), or null for built-in. */
  customTheme: string | null;
  /** CSS variable overrides for the active custom theme (P31). */
  customThemeCss: string;
  /** True once the first-run 3-step guide has been dismissed (P39). */
  hasSeenOnboarding: boolean;
  toggleTheme: () => void;
  setTheme: (t: Theme) => void;
  /** Apply a theme package: sets it active + stores its CSS for DOM injection. */
  applyCustomTheme: (name: string, css: string) => void;
  /** Revert to the built-in dark/light theme (P31). */
  clearCustomTheme: () => void;
  toggleSidebar: () => void;
  setSidebarCollapsed: (v: boolean) => void;
  /** Mark the first-run onboarding guide as seen (P39). */
  setHasSeenOnboarding: (v: boolean) => void;
}

function initialTheme(): Theme {
  if (typeof localStorage !== "undefined") {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "light" || saved === "dark") return saved;
  }
  // Default to dark — matches the flat, information-dense direction (§二).
  return "dark";
}

function initialCustomTheme(): string | null {
  if (typeof localStorage !== "undefined") {
    return localStorage.getItem(CUSTOM_KEY);
  }
  return null;
}

function initialOnboardingSeen(): boolean {
  if (typeof localStorage !== "undefined") {
    return localStorage.getItem(ONBOARDING_KEY) === "true";
  }
  return false;
}

export const useAppStore = create<AppState>()((set) => ({
  theme: initialTheme(),
  sidebarCollapsed: false,
  customTheme: initialCustomTheme(),
  customThemeCss: "",
  hasSeenOnboarding: initialOnboardingSeen(),
  toggleTheme: () =>
    set((s) => {
      const next: Theme = s.theme === "dark" ? "light" : "dark";
      localStorage?.setItem(STORAGE_KEY, next);
      return { theme: next };
    }),
  setTheme: (t) => {
    localStorage?.setItem(STORAGE_KEY, t);
    set({ theme: t });
  },
  applyCustomTheme: (name, css) => {
    localStorage?.setItem(CUSTOM_KEY, name);
    set({ customTheme: name, customThemeCss: css });
  },
  clearCustomTheme: () => {
    localStorage?.removeItem(CUSTOM_KEY);
    set({ customTheme: null, customThemeCss: "" });
  },
  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  setSidebarCollapsed: (v) => set({ sidebarCollapsed: v }),
  setHasSeenOnboarding: (v) => {
    localStorage?.setItem(ONBOARDING_KEY, v ? "true" : "false");
    set({ hasSeenOnboarding: v });
  },
}));
