// Workflow editor (P27 模式 C + P28 执行入 UI + P29 节点属性面板): React Flow
// canvas + toolbar + right-hand node property panel.
//  - Every edit auto-saves a draft after 500ms debounce (验收 #3).
//  - Cmd/Ctrl+S or the Save button writes the formal file atomically and
//    clears the draft (验收 #4).
//  - Save carries the load-time mtime; an external edit shifts it → conflict
//    prompt instead of silent overwrite (验收 #5).
//  - "运行" triggers the P17 engine via Tauri `run_workflow` (验收 #1): it
//    auto-saves the formal file first (F6-a), then loads + executes. Progress
//    arrives through `subscribeWorkflowRun` events (验收 #2); the final result
//    shows a summary + per-step input/output (验收 #3/#4/#5); history is shown
//    at the bottom (验收 #6/#7).
//  - Clicking a node opens the property panel (P29): edits write back to the
//    node's `data` (which round-trips into P17 `WorkflowStep`), and invalid
//    timeout/retry values block the formal save (验收 #4/#6).

import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import {
  addEdge,
  MarkerType,
  useEdgesState,
  useNodesState,
  type Connection,
  type Edge,
  type OnEdgesChange,
  type OnNodesChange,
} from "@xyflow/react";
import { AlertTriangle, ArrowLeft, Pause, Play, Plus, Save } from "lucide-react";
import { useCaspian, type WorkflowRunEvent } from "@/hooks/useCaspian";
import {
  docHasErrors,
  docToNodesEdges,
  nodesEdgesToDoc,
  type StepNode,
  type StepNodeData,
} from "@/lib/workflow";
import { NodePropertiesPanel } from "@/components/workflow/NodePropertiesPanel";
import type { RunRecord, RunResult, RunStatus, WorkflowDoc } from "@/types/workflow";
import { formatRelative } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { WorkflowCanvas } from "@/components/workflow/WorkflowCanvas";

type Status = "loading" | "idle" | "draft" | "saved" | "conflict";

type RunPhase = "idle" | "pending" | "running" | "completed" | "failed";

interface RunState {
  phase: RunPhase;
  runId?: string;
  result?: RunResult;
  error?: string;
}

// Engine `RunStatus` → flat status-dot color (no glow / no gradient).
const RUN_COLOR: Record<RunStatus, string> = {
  running: "var(--color-accent)",
  completed: "var(--color-success)",
  failed: "var(--color-danger)",
  skipped: "var(--color-neutral-400)",
  terminated: "var(--color-success)",
};

const RUN_LABEL: Record<RunStatus, string> = {
  running: "运行中",
  completed: "已完成",
  failed: "失败",
  skipped: "已跳过",
  terminated: "已完成（提前结束）",
};

function StatusBadge({ status }: { status: Status }) {
  const map: Record<Status, { label: string; cls: string }> = {
    loading: { label: "加载中", cls: "text-muted-foreground" },
    idle: { label: "已加载", cls: "text-muted-foreground" },
    draft: { label: "草稿已保存", cls: "text-accent" },
    saved: { label: "已保存", cls: "text-accent" },
    conflict: { label: "冲突", cls: "text-danger" },
  };
  const { label, cls } = map[status];
  return <span className={`text-[11px] ${cls}`}>{label}</span>;
}

function RunDot({ color }: { color: string }) {
  return (
    <span
      className="inline-block h-2 w-2 shrink-0 rounded-full"
      style={{ background: color }}
    />
  );
}

function runPhaseColor(phase: RunPhase): string {
  switch (phase) {
    case "pending":
      return "var(--color-neutral-400)";
    case "running":
      return "var(--color-accent)";
    case "completed":
      return "var(--color-success)";
    case "failed":
      return "var(--color-danger)";
    default:
      return "transparent";
  }
}

function runPhaseLabel(phase: RunPhase, result?: RunResult): string {
  switch (phase) {
    case "pending":
      return "等待执行…";
    case "running":
      return "运行中";
    case "completed":
      return result?.terminated ? "已完成（提前结束）" : "已完成";
    case "failed":
      return "运行失败";
    default:
      return "";
  }
}

export function WorkflowEditorPage() {
  const { name = "" } = useParams();
  const navigate = useNavigate();
  const capi = useCaspian();

  const [nodes, setNodes, onNodesChangeBase] = useNodesState<StepNode>([]);
  const [edges, setEdges, onEdgesChangeBase] = useEdgesState<Edge>([]);
  const [meta, setMeta] = useState<{ display_name?: string; description?: string }>({});
  const [status, setStatus] = useState<Status>("loading");
  const [error, setError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  const [run, setRun] = useState<RunState>({ phase: "idle" });
  const [history, setHistory] = useState<RunRecord[]>([]);
  const [expandedStep, setExpandedStep] = useState<string | null>(null);

  const loadedMtimeRef = useRef<number | undefined>(undefined);
  const draftTimer = useRef<number | null>(null);
  const savingRef = useRef(false);
  const nodesRef = useRef(nodes);
  const edgesRef = useRef(edges);
  const metaRef = useRef(meta);

  useEffect(() => {
    nodesRef.current = nodes;
  }, [nodes]);
  useEffect(() => {
    edgesRef.current = edges;
  }, [edges]);
  useEffect(() => {
    metaRef.current = meta;
  }, [meta]);

  // Currently selected node (drives the P29 property panel).
  const selected = nodes.find((n) => n.selected) ?? null;

  // Subscribe to run events (real Tauri or mock bus). Drives 验收 #2.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void capi
      .subscribeWorkflowRun((e: WorkflowRunEvent) => {
        if (e.type === "started") {
          setRun({ phase: "running", runId: e.run_id });
        } else if (e.type === "finished") {
          setRun({ phase: "completed", runId: e.result.run_id, result: e.result });
          void capi.listRuns(name).then(setHistory).catch(() => {});
        } else if (e.type === "errored") {
          setRun({ phase: "failed", runId: e.run_id, error: e.error });
          void capi.listRuns(name).then(setHistory).catch(() => {});
        }
      })
      .then((u) => {
        unlisten = u;
      });
    return () => unlisten?.();
  }, [capi, name]);

  // Load run history on mount (验收 #6/#7).
  useEffect(() => {
    void capi.listRuns(name).then(setHistory).catch(() => {});
  }, [capi, name]);

  // Load on mount (restores draft if present → refresh recovery, 验收 #3).
  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    (async () => {
      try {
        const res = await capi.loadWorkflow(name);
        if (cancelled) return;
        const { nodes: n, edges: e } = docToNodesEdges(res.doc);
        setNodes(n);
        setEdges(e);
        setMeta({ display_name: res.doc.display_name, description: res.doc.description });
        loadedMtimeRef.current = res.modified;
        setStatus("idle");
      } catch (err) {
        if (!cancelled) setError(String(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [name, capi, setNodes, setEdges]);

  // Per-node draft isolation (D3): cancelling a pending debounce on node switch
  // prevents a half-edited value from a previous node from flushing late.
  useEffect(() => {
    if (draftTimer.current) window.clearTimeout(draftTimer.current);
  }, [selected?.id]);

  const buildDoc = useCallback(
    (): WorkflowDoc =>
      nodesEdgesToDoc(name, metaRef.current, nodesRef.current, edgesRef.current),
    [name],
  );

  // 500ms debounced draft autosave (验收 #3). Paused during an explicit save (D4).
  const scheduleDraft = useCallback(() => {
    if (savingRef.current) return;
    setStatus("draft");
    if (draftTimer.current) window.clearTimeout(draftTimer.current);
    draftTimer.current = window.setTimeout(() => {
      void capi.saveWorkflowDraft(name, buildDoc()).catch(() => {});
    }, 500);
  }, [buildDoc, capi, name]);

  const handleNodesChange: OnNodesChange<StepNode> = (changes) => {
    onNodesChangeBase(changes);
    scheduleDraft();
  };
  const handleEdgesChange: OnEdgesChange = (changes) => {
    onEdgesChangeBase(changes);
    scheduleDraft();
  };
  const onConnect = useCallback(
    (c: Connection) => {
      setEdges((eds) =>
        addEdge({ ...c, markerEnd: { type: MarkerType.ArrowClosed } }, eds),
      );
      scheduleDraft();
    },
    [setEdges, scheduleDraft],
  );

  // P29: write an edited step's config back onto its node `data`. Triggers the
  // draft autosave (deferred 500ms, so it serializes the settled state).
  const updateNodeData = useCallback(
    (id: string, patch: Partial<StepNodeData>) => {
      setNodes((nds) =>
        nds.map((n) => (n.id === id ? { ...n, data: { ...n.data, ...patch } } : n)),
      );
      scheduleDraft();
    },
    [setNodes, scheduleDraft],
  );

  const addStep = useCallback(() => {
    const id = `step_${Date.now()}`;
    setNodes((nds) => [
      ...nds,
      {
        id,
        type: "step",
        position: { x: 160 + (nds.length % 4) * 40, y: 110 + nds.length * 130 },
        data: { skill: "new_skill" },
      },
    ]);
    scheduleDraft();
  }, [setNodes, scheduleDraft]);

  // Block the formal save when any step fails field validation (验收 #4/#6).
  const doSave = useCallback(
    async (force = false) => {
      const doc = buildDoc();
      if (docHasErrors(doc)) {
        setSaveError("存在非法字段（超时/重试超出范围），请修正后再保存");
        return;
      }
      setSaveError(null);
      savingRef.current = true;
      try {
        const res = await capi.saveWorkflow(
          name,
          doc,
          force ? undefined : loadedMtimeRef.current,
        );
        if (res.conflict) {
          setStatus("conflict");
          return;
        }
        loadedMtimeRef.current = res.modified;
        setStatus("saved");
      } finally {
        savingRef.current = false;
      }
    },
    [buildDoc, capi, name],
  );

  // Run: auto-save the formal file first (F6-a), then trigger the engine (验收 #1).
  const doRun = useCallback(async () => {
    setRun({ phase: "pending" });
    if (docHasErrors(buildDoc())) {
      setSaveError("存在非法字段（超时/重试超出范围），请修正后再运行");
      setRun({ phase: "idle" });
      return;
    }
    setSaveError(null);
    await doSave(false);
    const res = await capi.runWorkflow(name);
    setRun({ phase: "pending", runId: res.run_id });
  }, [buildDoc, capi, name, doSave]);

  // Cmd/Ctrl+S (验收 #4).
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void doSave(false);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [doSave]);

  if (status === "loading") {
    return <div className="p-6 text-[13px] text-muted-foreground">加载中…</div>;
  }
  if (error) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3">
        <p className="text-[13px] text-danger">加载失败：{error}</p>
        <Button variant="outline" size="sm" onClick={() => navigate("/workflows")}>
          <ArrowLeft size={14} />
          返回列表
        </Button>
      </div>
    );
  }

  const running = run.phase === "pending" || run.phase === "running";

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
        <Button
          variant="ghost"
          size="icon"
          aria-label="返回"
          title="返回列表"
          onClick={() => navigate("/workflows")}
        >
          <ArrowLeft size={16} />
        </Button>
        <Button variant="outline" size="sm" onClick={addStep}>
          <Plus size={14} />
          新建节点
        </Button>
        <Button variant="primary" size="sm" onClick={() => doSave(false)}>
          <Save size={14} />
          保存
        </Button>
        <Button
          variant="primary"
          size="sm"
          onClick={() => void doRun()}
          disabled={running}
        >
          {running ? <Pause size={14} /> : <Play size={14} />}
          {running ? "运行中…" : "运行"}
        </Button>
        <StatusBadge status={status} />
        {run.phase !== "idle" && (
          <span className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
            <RunDot color={runPhaseColor(run.phase)} />
            {runPhaseLabel(run.phase, run.result)}
          </span>
        )}
        <div className="ml-auto flex items-center gap-2">
          <Input
            value={meta.display_name ?? ""}
            placeholder="显示名称"
            onChange={(e) => {
              setMeta((m) => ({ ...m, display_name: e.target.value }));
              scheduleDraft();
            }}
            className="h-7 w-40 text-[12px]"
          />
          <Input
            value={meta.description ?? ""}
            placeholder="描述"
            onChange={(e) => {
              setMeta((m) => ({ ...m, description: e.target.value }));
              scheduleDraft();
            }}
            className="h-7 w-48 text-[12px]"
          />
        </div>
      </div>

      {saveError && (
        <div className="flex items-center gap-2 border-b border-border bg-muted px-4 py-1.5">
          <AlertTriangle size={14} className="shrink-0 text-danger" />
          <span className="text-[12px] text-danger">{saveError}</span>
        </div>
      )}

      <div className="flex min-h-0 flex-1">
        <div className="min-h-0 flex-1">
          <WorkflowCanvas
            nodes={nodes}
            edges={edges}
            onNodesChange={handleNodesChange}
            onEdgesChange={handleEdgesChange}
            onConnect={onConnect}
          />
        </div>

        {/* 节点属性面板（P29 验收 #1/#5） */}
        {selected && (
          <NodePropertiesPanel
            key={selected.id}
            node={selected}
            onChange={(patch) => updateNodeData(selected.id, patch)}
          />
        )}
      </div>

      {/* 运行结果（验收 #3/#4/#5） */}
      {run.phase === "completed" && run.result && (
        <div className="max-h-64 overflow-auto border-t border-border bg-muted px-4 py-3">
          <div className="mb-2 flex items-center gap-2">
            <RunDot color={runPhaseColor("completed")} />
            <span className="text-[12px] font-medium text-foreground">
              {runPhaseLabel("completed", run.result)}
            </span>
            <span className="text-[11px] text-muted-foreground">
              {run.result.steps.length} 步 · {run.result.duration_ms}ms
              {run.result.skipped_steps > 0 && ` · ${run.result.skipped_steps} 跳过`}
            </span>
          </div>
          <div className="space-y-1">
            {run.result.steps.map((s) => (
              <div key={s.step_id} className="rounded border border-border bg-card">
                <button
                  type="button"
                  className="flex w-full items-center gap-2 px-2 py-1.5 text-left text-[12px]"
                  onClick={() =>
                    setExpandedStep((cur) => (cur === s.step_id ? null : s.step_id))
                  }
                >
                  <RunDot color={RUN_COLOR.completed} />
                  <span className="font-mono text-[11px] text-foreground">{s.step_id}</span>
                  <span className="text-muted-foreground">· {s.skill}</span>
                  <span className="ml-auto text-[11px] text-muted-foreground">
                    {s.duration_ms}ms
                  </span>
                </button>
                {expandedStep === s.step_id && (
                  <pre className="selectable overflow-auto border-t border-border px-2 py-1.5 text-[11px] text-muted-foreground">
                    {JSON.stringify(s.output, null, 2)}
                  </pre>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {run.phase === "failed" && (
        <div className="flex items-center gap-3 border-t border-border bg-muted px-4 py-2.5">
          <RunDot color={runPhaseColor("failed")} />
          <AlertTriangle size={15} className="shrink-0 text-danger" />
          <span className="text-[12px] text-danger">运行失败：{run.error}</span>
        </div>
      )}

      {/* 最近运行（验收 #6/#7） */}
      {history.length > 0 && (
        <div className="border-t border-border px-4 py-2">
          <div className="mb-1 text-[11px] text-muted-foreground">最近运行</div>
          <div className="flex flex-wrap gap-2">
            {history.slice(0, 6).map((r) => (
              <span
                key={r.run_id}
                className="flex items-center gap-1.5 rounded border border-border bg-card px-2 py-0.5 text-[11px]"
              >
                <RunDot color={RUN_COLOR[r.status]} />
                <span className="text-foreground">{RUN_LABEL[r.status]}</span>
                <span className="text-muted-foreground">{formatRelative(r.started_at)}</span>
              </span>
            ))}
          </div>
        </div>
      )}

      {status === "conflict" && (
        <div className="flex items-center gap-3 border-t border-border bg-muted px-4 py-2.5">
          <AlertTriangle size={15} className="shrink-0 text-danger" />
          <span className="text-[12px] text-foreground">
            正式文件已被外部修改，保存会覆盖对方的更改。
          </span>
          <div className="ml-auto flex items-center gap-1.5">
            <Button variant="primary" size="sm" onClick={() => doSave(true)}>
              仍然覆盖
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={async () => {
                const res = await capi.loadWorkflow(name);
                const { nodes: n, edges: e } = docToNodesEdges(res.doc);
                setNodes(n);
                setEdges(e);
                loadedMtimeRef.current = res.modified;
                setStatus("idle");
              }}
            >
              放弃并重新加载
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
