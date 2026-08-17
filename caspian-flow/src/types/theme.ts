/**
 * Theme package types (P31 · A3).
 *
 * Mirrors the Rust `theme` module: a theme package is a directory under
 * `~/.caspian/themes/<name>/` with `manifest.yaml` + `theme.css`. The UI lists
 * installed packages, shows load issues, and applies a package by injecting its
 * CSS variable overrides + flipping `document.documentElement[data-theme]`.
 */

/** A single problem discovered while scanning a theme package. */
export interface ThemeIssue {
  kind: "missing_manifest" | "read_error" | "parse_error" | "validation_error";
  /** Package directory path on disk. */
  path: string;
  /** Package name if it could be determined. */
  name?: string;
  /** Human-readable, user-comprehensible reason (Chinese). */
  reason: string;
}

/** Lightweight metadata for a theme package shown in the picker. */
export interface ThemeMeta {
  name: string;
  author?: string;
  version: string;
  /** Whether this theme is currently active. */
  active: boolean;
}

/** Result of `list_themes` — themes + any load issues. */
export interface ThemeListResult {
  themes: ThemeMeta[];
  issues: ThemeIssue[];
}

/** Payload of the `theme_changed` event (broadcast on apply / disk change). */
export interface ThemeChanged {
  name: string | null;
  css: string;
}
