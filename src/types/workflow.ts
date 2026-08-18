// Workflow canvas domain types (P27, 模式 C).
//
// The editor holds a `WorkflowDoc` — a JSON document that round-trips to the
// P17 `workflow.yaml` on disk. The `ui` section carries React Flow node/edge
// layout and is ignored by the P17 engine (its `Workflow` struct has no
// `deny_unknown_fields`), so the canvas never pollutes execution.

/** A single step as edited in the canvas.
 *
 * Field names mirror P17 `WorkflowStep` (`schema.rs:127`) **exactly** so the
 * JSON→YAML passthrough in `save_workflow` emits loadable YAML and the edits
 * actually reach `WorkflowEngine::execute` (P28). Do NOT add P17-unknown keys
 * here — serde has no `deny_unknown_fields`, so unknown keys would be silently
 * dropped on load (breaking the round-trip and 验收 #6). */
export interface WorkflowStepDoc {
  id: string;
  skill: string;
  /** P17 `input` — the skill's input params (template expressions). */
  input?: Record<string, unknown>;
  /** P17 `output` — a single output variable name (not an object). */
  output?: string;
  /** P17 `condition` — skip-if-false expression. */
  condition?: string;
  /** P17 `timeout` (seconds, u64). */
  timeout?: number;
  /** P17 `retry_count` (usize) — doc calls this `retry`. */
  retry_count?: number;
  depends_on?: string[];
}

/** A node's persisted layout (mirrors a React Flow node). */
export interface CanvasNode {
  id: string;
  skill: string;
  x: number;
  y: number;
}

/** A dependency edge (target depends on source). */
export interface CanvasEdge {
  id: string;
  source: string;
  target: string;
}

/** The full document serialized to disk / IPC. */
export interface WorkflowDoc {
  schema_version?: string;
  name: string;
  display_name?: string;
  description?: string;
  steps: WorkflowStepDoc[];
  ui: {
    nodes: CanvasNode[];
    edges: CanvasEdge[];
  };
}

/** A row in the workflow list view. `name` is the directory identity. */
export interface WorkflowListEntry {
  name: string;
  display_name: string;
  description: string;
  modified: number;
  step_count: number;
}

/** Result of loading a workflow: JSON doc + recorded mtime for conflict checks. */
export interface WorkflowLoadResult {
  doc: WorkflowDoc;
  modified: number;
}

/** Save outcome (real Tauri path) — the new mtime of the formal file. */
export interface SaveResult {
  modified: number;
  conflict?: boolean;
}

// --- P28: execution ----------------------------------------------------------

/** Engine lifecycle status (mirrors P17 `RunStatus`, snake_case). */
export type RunStatus = "running" | "completed" | "failed" | "skipped" | "terminated";

/** Synchronous handle returned by `runWorkflow`. */
export interface RunResponse {
  run_id: string;
  status: RunStatus;
}

/** A node-level execution result (验收 #5). */
export interface RunStepResult {
  step_id: string;
  skill: string;
  output: unknown;
  duration_ms: number;
}

/** Final run result (验收 #3/#4/#5). */
export interface RunResult {
  run_id: string;
  workflow_name: string;
  status: RunStatus;
  duration_ms: number;
  terminated: boolean;
  skipped_steps: number;
  steps: RunStepResult[];
  outputs: Record<string, unknown>;
}

/** A persisted run record (验收 #6/#7). */
export interface RunRecord {
  run_id: string;
  workflow_name: string;
  status: RunStatus;
  started_at: number;
  finished_at: number | null;
}
