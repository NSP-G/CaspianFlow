// Workflow canvas document ⇄ React Flow conversion (P27, 模式 C) + P29 field map.
//
// The on-disk document is a `WorkflowDoc` (JSON → P17 YAML). The canvas edits
// `nodes`/`edges` (React Flow). We translate between the two on save/load.
//
// P29: node property fields live on the step and are carried through to P17's
// `WorkflowStep` (schema.rs:127) using the **exact same field names**, so the
// `save_workflow` JSON→YAML passthrough emits loadable YAML and the edits reach
// `WorkflowEngine::execute` (P28). See `STEP_FIELDS` for the single source of truth.

import { MarkerType } from "@xyflow/react";
import type { Edge, Node } from "@xyflow/react";
import type { CanvasEdge, CanvasNode, WorkflowDoc, WorkflowStepDoc } from "@/types/workflow";

/** Per-step editable config carried on a React Flow node (P29). */
export interface StepNodeData {
  skill: string;
  /** P17 `input` — skill input params (template expressions). */
  input?: Record<string, unknown>;
  /** P17 `output` — single output variable name. */
  output?: string;
  /** P17 `condition` — skip-if-false expression. */
  condition?: string;
  /** P17 `timeout` (seconds). */
  timeout?: number;
  /** P17 `retry_count`. */
  retry_count?: number;
  [key: string]: unknown;
}

/** A React Flow node carrying a skill + its editable config. */
export type StepNode = Node<StepNodeData, "step">;

/** Single source of truth for the node property panel (P29 §四 D1).
 *
 * `p17` MUST equal a real `WorkflowStep` serde field name — emitting any other
 * key would be silently dropped by `Workflow::load` (no `deny_unknown_fields`).
 * `control` drives which editor the panel renders; `min`/`max` feed validation. */
export interface StepFieldSpec {
  p17: keyof WorkflowStepDoc;
  label: string;
  hint: string;
  control: "input-json" | "output" | "condition" | "timeout" | "retry";
  min?: number;
  max?: number;
}

export const STEP_FIELDS: StepFieldSpec[] = [
  {
    p17: "input",
    label: "输入参数 (input)",
    hint: "技能入参，模板表达式如 ${variables.x}；可用 JSON 模式写结构化值",
    control: "input-json",
  },
  {
    p17: "output",
    label: "输出变量名 (output)",
    hint: "本步骤结果写入的变量名，后续步骤以 ${steps.<id>.output} 引用",
    control: "output",
  },
  {
    p17: "condition",
    label: "条件 (condition)",
    hint: "为假则跳过本步骤，如 ${steps.a.output.n} > 1000",
    control: "condition",
  },
  {
    p17: "timeout",
    label: "超时 (timeout)",
    hint: "本步骤超时（秒），留空使用工作流默认 300",
    control: "timeout",
    min: 1,
    max: 300,
  },
  {
    p17: "retry_count",
    label: "重试 (retry_count)",
    hint: "本步骤重试次数，留空使用工作流 error_handling 配置",
    control: "retry",
    min: 0,
    max: 5,
  },
];

export const TIMEOUT_MIN = 1;
export const TIMEOUT_MAX = 300;
export const RETRY_MIN = 0;
export const RETRY_MAX = 5;

/** Validate a `timeout` value (seconds). `undefined`/"" means "unset" (valid). */
export function validateTimeout(v: unknown): string | null {
  if (v === undefined || v === null || v === "") return null;
  const n = Number(v);
  if (!Number.isFinite(n) || !Number.isInteger(n)) return "超时必须为整数秒";
  if (n < TIMEOUT_MIN || n > TIMEOUT_MAX) return `超时范围 ${TIMEOUT_MIN}–${TIMEOUT_MAX} 秒`;
  return null;
}

/** Validate a `retry_count` value. `undefined`/"" means "unset" (valid). */
export function validateRetry(v: unknown): string | null {
  if (v === undefined || v === null || v === "") return null;
  const n = Number(v);
  if (!Number.isFinite(n) || !Number.isInteger(n)) return "重试次数必须为整数";
  if (n < RETRY_MIN || n > RETRY_MAX) return `重试范围 ${RETRY_MIN}–${RETRY_MAX} 次`;
  return null;
}

/** Validate the `input` JSON text (must parse to an object). */
export function validateInputJson(text: string): string | null {
  const t = text.trim();
  if (t === "") return null;
  try {
    const v = JSON.parse(t);
    if (typeof v !== "object" || v === null || Array.isArray(v)) {
      return "input 必须是 JSON 对象";
    }
    return null;
  } catch {
    return "input 不是合法 JSON";
  }
}

/** Whether a whole document has any step-level validation error (blocks save). */
export function docHasErrors(doc: WorkflowDoc): boolean {
  return doc.steps.some(
    (s) => validateTimeout(s.timeout) !== null || validateRetry(s.retry_count) !== null,
  );
}

/** A brand-new workflow: one starter step so the canvas isn't empty. */
export function blankDoc(name: string): WorkflowDoc {
  return {
    schema_version: "1.0",
    name,
    display_name: name,
    description: "",
    steps: [{ id: "step_1", skill: "read_file", depends_on: [] }],
    ui: {
      nodes: [{ id: "step_1", skill: "read_file", x: 140, y: 96 }],
      edges: [],
    },
  };
}

/** Build React Flow nodes/edges from a stored document. If `ui` is missing
 * (legacy/empty), synthesize nodes from the `steps` list. Per-step config
 * (input/output/condition/timeout/retry_count) is merged into node `data`. */
export function docToNodesEdges(doc: WorkflowDoc): {
  nodes: StepNode[];
  edges: Edge[];
} {
  const stepById = new Map<string, WorkflowStepDoc>();
  for (const s of doc.steps) stepById.set(s.id, s);

  const uiNodes = doc.ui?.nodes ?? [];
  const uiEdges = doc.ui?.edges ?? [];

  let nodes: StepNode[];
  if (uiNodes.length > 0) {
    nodes = uiNodes.map((n: CanvasNode) => {
      const s = stepById.get(n.id);
      return {
        id: n.id,
        type: "step",
        position: { x: n.x, y: n.y },
        data: {
          skill: n.skill,
          input: s?.input,
          output: s?.output,
          condition: s?.condition,
          timeout: s?.timeout,
          retry_count: s?.retry_count,
        },
      };
    });
  } else {
    // Synthesize a vertical column from the step list.
    nodes = doc.steps.map((s: WorkflowStepDoc, i: number) => ({
      id: s.id,
      type: "step",
      position: { x: 140, y: 96 + i * 120 },
      data: {
        skill: s.skill,
        input: s.input,
        output: s.output,
        condition: s.condition,
        timeout: s.timeout,
        retry_count: s.retry_count,
      },
    }));
  }

  const edges: Edge[] = uiEdges.map((e: CanvasEdge) => ({
    id: e.id,
    source: e.source,
    target: e.target,
    markerEnd: { type: MarkerType.ArrowClosed },
  }));

  return { nodes, edges };
}

/** Serialize the current canvas back into a `WorkflowDoc`.
 * Edge direction: `source → target` means the target step depends on the source.
 * Per-step config is carried from node `data` into `steps[]` so it round-trips
 * to P17 `WorkflowStep`. */
export function nodesEdgesToDoc(
  name: string,
  meta: { display_name?: string; description?: string },
  nodes: StepNode[],
  edges: Edge[],
): WorkflowDoc {
  const steps = new Map<string, WorkflowStepDoc>();
  for (const n of nodes) {
    steps.set(n.id, {
      id: n.id,
      skill: (n.data?.skill as string) || "new_skill",
      input: n.data?.input,
      output: n.data?.output,
      condition: n.data?.condition,
      timeout: n.data?.timeout,
      retry_count: n.data?.retry_count,
      depends_on: [],
    });
  }
  for (const e of edges) {
    const target = steps.get(e.target);
    if (target) {
      target.depends_on = [...(target.depends_on ?? []), e.source];
    }
  }

  return {
    schema_version: "1.0",
    name,
    display_name: meta.display_name ?? name,
    description: meta.description ?? "",
    steps: Array.from(steps.values()),
    ui: {
      nodes: nodes.map((n) => ({
        id: n.id,
        skill: (n.data?.skill as string) || "new_skill",
        x: Math.round(n.position.x),
        y: Math.round(n.position.y),
      })),
      edges: edges.map((e) => ({
        id: e.id,
        source: e.source,
        target: e.target,
      })),
    },
  };
}
