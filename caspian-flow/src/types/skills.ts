/**
 * Skill domain types (P26 §二 页面一). Hand-written stand-ins for the ts-rs
 * generated types — same convention as `chat.ts`. Replaced by
 * `src/types/generated/*` once the P29 Skill SDK / real interface lands.
 */

/** Coarse category used for the filter chips (P26 §二.1.5). */
export type SkillCategory = "file" | "shell" | "network" | "text" | "agent";

export const SKILL_CATEGORIES: { id: SkillCategory; label: string }[] = [
  { id: "file", label: "文件" },
  { id: "shell", label: "命令" },
  { id: "network", label: "网络" },
  { id: "text", label: "文本" },
  { id: "agent", label: "智能体" },
];

export interface Skill {
  id: string;
  /** Machine name shown in mono (e.g. `read_file`). */
  name: string;
  description: string;
  category: SkillCategory;
  /** Toggle state (mock-managed in P26; real in P29). */
  enabled: boolean;
  /** Human-readable parameter schema (shown in the detail panel). */
  schema: string;
  /** Capabilities the skill requires (shown in the detail panel). */
  permissions: string[];
  /** Example trigger phrases (shown in the detail panel). */
  triggers: string[];
}

// --- P30 WS1: module resilience observability ---

/** Why a skill directory failed to load. Mirrors the Rust `ScanIssueKind`. */
export type ModuleIssueKind =
  | "missing_manifest"
  | "read_error"
  | "parse_error"
  | "validation_error";

/** A single missing/broken module, reported so the UI can be explicit. */
export interface ModuleIssue {
  kind: ModuleIssueKind;
  path: string;
  skill_name?: string | null;
  reason: string;
}

/** Snapshot of the skills directory: loaded skills + any issues (P30 WS1 §3). */
export interface ModuleStatus {
  skills: Skill[];
  issues: ModuleIssue[];
  scanned_dirs: number;
}
