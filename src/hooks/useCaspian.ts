import type { AgentStatusEvent, Session, StreamChunk } from "@/types/chat";
import type { KnowledgeDocument } from "@/types/knowledge";
import type { ModuleIssue, ModuleStatus, Skill, SkillCategory } from "@/types/skills";
import type { ThemeChanged, ThemeListResult, ThemeMeta } from "@/types/theme";
import type {
  RunRecord,
  RunResponse,
  RunResult,
  RunStatus,
  SaveResult,
  WorkflowDoc,
  WorkflowListEntry,
  WorkflowLoadResult,
} from "@/types/workflow";
import { MOCK_DATA_PATH } from "@/lib/constants";

/**
 * Tauri IPC wrapper (P25 §九).
 *
 * Two paths:
 *  - MOCK (browser / vite preview / sandbox): TS-simulated. `sendMessage`
 *    drives the status machine + chunked stream through callbacks so the UI is
 *    fully demonstrable without a Rust runtime.
 *  - REAL (inside the Tauri webview, set by `src-tauri`): delegates to
 *    `@tauri-apps/api` `invoke` + event `listen`. The commands/events are the
 *    mock `#[tauri::command]`s produced in P25 and swapped for real ones later.
 */

export interface SendHandlers {
  onStatus?: (e: AgentStatusEvent) => void;
  onChunk?: (e: StreamChunk) => void;
}

function runningInTauri(): boolean {
  return (
    typeof window !== "undefined" &&
    "__TAURI_INTERNALS__" in (window as unknown as Record<string, unknown>)
  );
}

const MOCK_SESSIONS: Session[] = [
  { id: "s_demo", title: "工作流引擎怎么实现的？", updatedAt: Date.now() - 1000 * 60 * 3 },
  { id: "s_2", title: "本地的数据存在哪里", updatedAt: Date.now() - 1000 * 60 * 42 },
  { id: "s_3", title: "帮我总结这段记忆", updatedAt: Date.now() - 1000 * 60 * 60 * 5 },
];

function mockAnswer(text: string): string {
  return (
    `收到：「${text.trim()}」\n\n` +
    `（P25 演示响应）CaspianFlow 以本地优先方式运行，所有数据落在 ~/.caspian/。` +
    `当前通道为前端 mock，真实推理将在 P21/P22 模块就绪后经 Tauri command 接入。`
  );
}

const MOCK_SKILLS: Skill[] = [
  {
    id: "sk_read_file",
    name: "read_file",
    description: "读取本地文件内容并返回文本。",
    category: "file",
    enabled: true,
    schema: "args: { path: string, start?: number, end?: number }",
    permissions: ["读取文件系统"],
    triggers: ["读一下这个文件", "打开 ~/.caspian/SOUL.md"],
  },
  {
    id: "sk_write_file",
    name: "write_file",
    description: "将文本写入本地文件，可创建或覆盖。",
    category: "file",
    enabled: false,
    schema: "args: { path: string, content: string, append?: boolean }",
    permissions: ["写入文件系统"],
    triggers: ["把这段保存成 note.md", "帮我写个文件"],
  },
  {
    id: "sk_shell_command",
    name: "shell_command",
    description: "在沙箱中执行 shell 命令并捕获输出。",
    category: "shell",
    enabled: true,
    schema: "args: { command: string, cwd?: string, timeout_ms?: number }",
    permissions: ["执行命令", "读取环境变量"],
    triggers: ["跑一下 ls", "执行 pytest"],
  },
  {
    id: "sk_http_request",
    name: "http_request",
    description: "发起 HTTP 请求并解析响应。",
    category: "network",
    enabled: true,
    schema: "args: { method: string, url: string, headers?: object, body?: string }",
    permissions: ["网络访问"],
    triggers: ["请求这个接口", "抓一下网页"],
  },
  {
    id: "sk_summarize_text",
    name: "summarize_text",
    description: "对长文本做摘要，可选压缩比。",
    category: "text",
    enabled: false,
    schema: "args: { text: string, ratio?: number }",
    permissions: ["使用语言模型"],
    triggers: ["总结这段", "帮我提炼要点"],
  },
  {
    id: "sk_search_web",
    name: "search_web",
    description: "联网检索并汇总结果来源。",
    category: "network",
    enabled: false,
    schema: "args: { query: string, top_k?: number }",
    permissions: ["网络访问", "使用语言模型"],
    triggers: ["搜一下", "查查最新资料"],
  },
];

// --- P31 · A3: mock theme package (sandbox-demonstrable) --------------------
const MOCK_THEME_CSS = `:root {
  --color-accent: #c2410c;
  --background: #1a1410;
  --foreground: #f0e9e0;
  --border: #3a2e24;
}`;

// --- P30 WS1: real (Rust) Skill → UI Skill mapping --------------------------
// The real `list_skills` returns the Rust `Skill` shape, whose `category` is a
// free-form string and which has no `id`/`schema`/`permissions`/`triggers`
// fields the UI expects. Map it defensively so the marketplace renders without
// depending on the (not-yet-generated) ts-rs types.

const CAT_MAP: Record<string, SkillCategory> = {
  "file-system": "file",
  filesystem: "file",
  file: "file",
  shell: "shell",
  command: "shell",
  network: "network",
  text: "text",
  agent: "agent",
};

function mapRustSkill(s: unknown): Skill {
  const raw = (s ?? {}) as Record<string, unknown>;
  const runtime = (raw.runtime ?? {}) as Record<string, unknown>;
  const cat = CAT_MAP[String(raw.category ?? "")] ?? "agent";
  return {
    id: String(raw.name ?? ""),
    name: String(raw.name ?? ""),
    description: String(raw.description ?? ""),
    category: cat as SkillCategory,
    enabled: raw.enabled !== false,
    schema: `${String(runtime.runtime_type ?? "unknown")} · ${String(runtime.entry ?? "")}`,
    permissions: [],
    triggers: Array.isArray(raw.trigger_phrases) ? (raw.trigger_phrases as string[]) : [],
  };
}

const MOCK_DOCUMENTS: KnowledgeDocument[] = [
  {
    id: "doc_wf",
    name: "workflow-engine.md",
    importedAt: Date.now() - 1000 * 60 * 60 * 26,
    chunkCount: 18,
  },
  {
    id: "doc_soul",
    name: "SOUL.md",
    importedAt: Date.now() - 1000 * 60 * 60 * 50,
    chunkCount: 7,
  },
  {
    id: "doc_notes",
    name: "meeting-notes.txt",
    importedAt: Date.now() - 1000 * 60 * 60 * 73,
    chunkCount: 11,
  },
  {
    id: "doc_api",
    name: "api-reference.md",
    importedAt: Date.now() - 1000 * 60 * 60 * 96,
    chunkCount: 24,
  },
];

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

// --- Workflow canvas mock storage (P27, 模式 C) -----------------------------
// LocalStorage fallback used outside the Tauri webview. Mirrors the Rust
// manifest layout: formal doc under `caspian.wf.<name>`, draft under
// `caspian.wfdraft.<name>`. The draft is restored on load so a refresh after
// auto-save recovers the in-progress canvas (验收 #3).

const WF_PREFIX = "caspian.wf.";
const WF_DRAFT_PREFIX = "caspian.wfdraft.";

interface StoredWorkflow {
  doc: WorkflowDoc;
  modified: number;
}

function lsGet(key: string): StoredWorkflow | null {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as StoredWorkflow) : null;
  } catch {
    return null;
  }
}

function lsSet(key: string, val: StoredWorkflow): void {
  try {
    localStorage.setItem(key, JSON.stringify(val));
  } catch {
    /* ignore quota / serialization errors */
  }
}

// --- P28: mock run event bus + history (browser fallback) -------------------
// Mirrors the real Tauri event stream (`workflow_run_started` /
// `workflow_run_finished` / `workflow_run_errored`). The mock command drives
// these through a tiny in-module emitter so the UI lifecycle is demonstrable
// without a Rust runtime.

export type WorkflowRunEvent =
  | { type: "started"; run_id: string; workflow_name: string; started_at: number }
  | { type: "finished"; result: RunResult }
  | { type: "errored"; run_id: string; error: string };

const wfRunListeners = new Set<(e: WorkflowRunEvent) => void>();
const mockRuns = new Map<string, RunRecord>();

function emitWorkflowRun(e: WorkflowRunEvent): void {
  wfRunListeners.forEach((h) => h(e));
}

function mockRunId(): string {
  return `run_${Date.now().toString(16)}`;
}

function mockListWorkflows(): WorkflowListEntry[] {
  const out: WorkflowListEntry[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const key = localStorage.key(i);
    if (!key || !key.startsWith(WF_PREFIX)) continue;
    const stored = lsGet(key);
    if (!stored) continue;
    const name = key.slice(WF_PREFIX.length);
    out.push({
      name,
      display_name: stored.doc.display_name ?? stored.doc.name,
      description: stored.doc.description ?? "",
      modified: stored.modified,
      step_count: stored.doc.steps.length,
    });
  }
  return out.sort((a, b) => b.modified - a.modified);
}

export const caspian = {
  /** True when inside the Tauri webview (real IPC available). */
  isTauri: runningInTauri,

  /**
   * Send a user message. In mock mode, simulates THINKING → STREAMING_ANSWER
   * → IDLE and pushes chunked tokens through `handlers`. In real mode, fires
   * the `send_message` command and lets event subscriptions drive the UI.
   */
  async sendMessage(
    sessionId: string,
    text: string,
    handlers: SendHandlers = {},
  ): Promise<void> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("send_message", { sessionId, text });
      return;
    }

    handlers.onStatus?.({ session_id: sessionId, status: "THINKING", label: "规划任务" });
    await sleep(500);
    handlers.onStatus?.({
      session_id: sessionId,
      status: "STREAMING_ANSWER",
      label: "回答中",
    });
    const full = mockAnswer(text);
    // Stream by sentence/word to exercise the pre-allocated container (no jitter).
    const parts = full.match(/[^\n]+(\n)?/g) ?? [full];
    for (const part of parts) {
      await sleep(120);
      handlers.onChunk?.({ session_id: sessionId, chunk: part });
    }
    handlers.onStatus?.({ session_id: sessionId, status: "IDLE", label: "就绪" });
  },

  /** List conversations. Mock returns three placeholders (P25 验收 #2). */
  async listSessions(): Promise<Session[]> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<Session[]>("list_sessions");
    }
    await sleep(80);
    return MOCK_SESSIONS;
  },

  /** Data directory shorthand shown in the sidebar (P25 §六 / 验收 #5). */
  async getDataPath(): Promise<string> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<string>("get_data_path");
    }
    await sleep(40);
    return MOCK_DATA_PATH;
  },

  /**
   * Subscribe to agent status / stream events. Real mode uses Tauri `listen`;
   * mock mode is a no-op because `sendMessage` delivers via callbacks.
   * Returns an unlisten function.
   */
  async subscribe(
    onStatus: (e: AgentStatusEvent) => void,
    onChunk: (e: StreamChunk) => void,
  ): Promise<() => void> {
    if (!runningInTauri()) return () => {};
    const { listen } = await import("@tauri-apps/api/event");
    const u1 = await listen<AgentStatusEvent>("agent_status", (ev) =>
      onStatus(ev.payload),
    );
    const u2 = await listen<StreamChunk>("chat_stream_chunk", (ev) =>
      onChunk(ev.payload),
    );
    return () => {
      u1();
      u2();
    };
  },

  /**
   * List available skills (P26 页面一). Mock returns the in-memory array;
   * real mode will `invoke("list_skills")`. State is module-level so a toggle
   * persists across navigation within a session.
   */
  async listSkills(): Promise<Skill[]> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      const raw = await invoke<unknown[]>("list_skills");
      return raw.map(mapRustSkill);
    }
    await sleep(80);
    return MOCK_SKILLS.map((s) => ({ ...s }));
  },

  /** Toggle a skill's enabled state (P26 §二.1.3). Mock mutates in place. */
  async toggleSkill(id: string): Promise<Skill[]> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      try {
        const raw = await invoke<unknown[]>("toggle_skill", { id });
        return raw.map(mapRustSkill);
      } catch (e) {
        // P26 TODO: `toggle_skill` command not yet registered. Fall back to a
        // fresh list so the UI never breaks (P30 WS1 resilience).
        console.warn("[caspian] toggle_skill unavailable, falling back:", e);
        return this.listSkills();
      }
    }
    const target = MOCK_SKILLS.find((s) => s.id === id);
    if (target) target.enabled = !target.enabled;
    await sleep(40);
    return MOCK_SKILLS.map((s) => ({ ...s }));
  },

  /**
   * Module status: loaded skills + any missing/broken issues (P30 WS1 §3).
   * Drives the resilience banner. Real mode reads `get_module_status`; mock
   * returns an empty (healthy) status.
   */
  async getModuleStatus(): Promise<ModuleStatus> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      const raw = await invoke<{
        skills: unknown[];
        issues: ModuleIssue[];
        scanned_dirs: number;
      }>("get_module_status");
      return {
        skills: raw.skills.map(mapRustSkill),
        issues: raw.issues,
        scanned_dirs: raw.scanned_dirs,
      };
    }
    await sleep(40);
    return { skills: [], issues: [], scanned_dirs: 0 };
  },

  /**
   * Re-scan skills from disk and return the refreshed list (P26/P30).
   * Real mode fires `reload_skills`; mock returns the in-memory array.
   */
  async reloadSkills(): Promise<Skill[]> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("reload_skills");
      return this.listSkills();
    }
    await sleep(40);
    return MOCK_SKILLS.map((s) => ({ ...s }));
  },

  /** List imported knowledge documents (P26 页面二). Mock returns in-memory. */
  async listDocuments(): Promise<KnowledgeDocument[]> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<KnowledgeDocument[]>("list_documents");
    }
    await sleep(80);
    return MOCK_DOCUMENTS.map((d) => ({ ...d }));
  },

  /** Remove a document by id (P26 §二.2.3). Mock mutates in place. */
  async deleteDocument(id: string): Promise<KnowledgeDocument[]> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<KnowledgeDocument[]>("delete_document", { id });
    }
    const idx = MOCK_DOCUMENTS.findIndex((d) => d.id === id);
    if (idx >= 0) MOCK_DOCUMENTS.splice(idx, 1);
    await sleep(40);
    return MOCK_DOCUMENTS.map((d) => ({ ...d }));
  },

  /**
   * Import a document (P26 验收 #5). Mock appends a row keyed by the selected
   * file name; the real backend (P22) will chunk + embed. `chunkCount` is a
   * placeholder estimate.
   */
  async importDocument(name: string): Promise<KnowledgeDocument[]> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<KnowledgeDocument[]>("import_document", { name });
    }
    MOCK_DOCUMENTS.unshift({
      id: `doc_${Date.now()}`,
      name,
      importedAt: Date.now(),
      chunkCount: 1 + Math.floor(Math.random() * 12),
    });
    await sleep(60);
    return MOCK_DOCUMENTS.map((d) => ({ ...d }));
  },

  /**
   * List workflow definitions for the canvas list view (P27 验收 #1).
   * Mock enumerates the localStorage formal-doc keys.
   */
  async listWorkflows(): Promise<WorkflowListEntry[]> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<WorkflowListEntry[]>("list_workflows");
    }
    await sleep(80);
    return mockListWorkflows();
  },

  /**
   * Load a workflow by its directory name. Restores the draft if present so a
   * refresh after auto-save recovers the in-progress canvas (验收 #3).
   */
  async loadWorkflow(name: string): Promise<WorkflowLoadResult> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      const res = await invoke<{ doc: string; modified: number }>("load_workflow", {
        name,
      });
      return {
        doc: JSON.parse(res.doc) as WorkflowDoc,
        modified: res.modified,
      };
    }
    await sleep(60);
    const draft = lsGet(WF_DRAFT_PREFIX + name);
    const formal = lsGet(WF_PREFIX + name);
    const stored = draft ?? formal;
    if (!stored) throw new Error(`workflow not found: ${name}`);
    return { doc: stored.doc, modified: stored.modified };
  },

  /**
   * Explicitly save a workflow (atomic write in the real backend, draft
   * cleared). `expectedMtime` enables conflict detection against an external
   * edit (验收 #4/#5); a mismatch yields `{ conflict: true }`.
   */
  async saveWorkflow(
    name: string,
    doc: WorkflowDoc,
    expectedMtime?: number,
  ): Promise<SaveResult> {
    const docStr = JSON.stringify(doc);
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      try {
        const mtime = await invoke<number>("save_workflow", {
          name,
          doc: docStr,
          expected_mtime: expectedMtime ?? null,
        });
        return { modified: mtime };
      } catch (e) {
        if (String(e).includes("Conflict")) {
          return { modified: expectedMtime ?? 0, conflict: true };
        }
        throw e;
      }
    }
    await sleep(60);
    const existing = lsGet(WF_PREFIX + name);
    if (expectedMtime != null && existing && existing.modified !== expectedMtime) {
      return { modified: existing.modified, conflict: true };
    }
    const modified = Date.now();
    lsSet(WF_PREFIX + name, { doc, modified });
    try {
      localStorage.removeItem(WF_DRAFT_PREFIX + name);
    } catch {
      /* ignore */
    }
    return { modified };
  },

  /** Write a draft (auto-save, debounced by the caller). Isolated from formal. */
  async saveWorkflowDraft(name: string, doc: WorkflowDoc): Promise<void> {
    const docStr = JSON.stringify(doc);
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("save_workflow_draft", { name, doc: docStr });
      return;
    }
    await sleep(20);
    const existing = lsGet(WF_PREFIX + name);
    lsSet(WF_DRAFT_PREFIX + name, { doc, modified: existing?.modified ?? Date.now() });
  },

  /** Delete a workflow definition and any stale draft. */
  async deleteWorkflow(name: string): Promise<void> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("delete_workflow", { name });
      return;
    }
    await sleep(40);
    try {
      localStorage.removeItem(WF_PREFIX + name);
      localStorage.removeItem(WF_DRAFT_PREFIX + name);
    } catch {
      /* ignore */
    }
  },

  /**
   * Trigger a workflow run (验收 #1). In real mode, fires `run_workflow` and
   * returns the run handle synchronously; progress arrives via
   * `subscribeWorkflowRun`. In mock mode, simulates the lifecycle
   * (started → finished) over the same event bus so the UI is identical.
   */
  async runWorkflow(name: string): Promise<RunResponse> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<RunResponse>("run_workflow", { name });
    }
    const run_id = mockRunId();
    const started_at = Date.now();
    emitWorkflowRun({
      type: "started",
      run_id,
      workflow_name: name,
      started_at,
    });
    const stored = lsGet(WF_PREFIX + name) ?? lsGet(WF_DRAFT_PREFIX + name);
    const steps = (stored?.doc.steps ?? []).map((s) => ({
      step_id: s.id,
      skill: s.skill,
      output: { ok: true },
      duration_ms: 80 + Math.floor(Math.random() * 240),
    }));
    setTimeout(() => {
      const result: RunResult = {
        run_id,
        workflow_name: name,
        status: "completed",
        duration_ms: steps.reduce((a, s) => a + s.duration_ms, 0),
        terminated: false,
        skipped_steps: 0,
        steps,
        outputs: {},
      };
      mockRuns.set(run_id, {
        run_id,
        workflow_name: name,
        status: "completed",
        started_at,
        finished_at: Date.now(),
      });
      emitWorkflowRun({ type: "finished", result });
    }, 900);
    return { run_id, status: "running" };
  },

  /** Current status of a run (验收 #2/#6). */
  async getRunStatus(runId: string): Promise<RunStatus> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<RunStatus>("get_run_status", { runId });
    }
    return mockRuns.get(runId)?.status ?? "running";
  },

  /** List run history, optionally filtered by workflow (验收 #6/#7). */
  async listRuns(workflowName?: string): Promise<RunRecord[]> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<RunRecord[]>("list_runs", { workflowName: workflowName ?? null });
    }
    const all = [...mockRuns.values()].sort((a, b) => b.started_at - a.started_at);
    return workflowName ? all.filter((r) => r.workflow_name === workflowName) : all;
  },

  /**
   * Subscribe to workflow run events. Real mode uses Tauri `listen` for
   * `workflow_run_started` / `workflow_run_finished` / `workflow_run_errored`;
   * mock mode routes the in-module emitter. Returns an unlisten function.
   */
  async subscribeWorkflowRun(
    handler: (e: WorkflowRunEvent) => void,
  ): Promise<() => void> {
    if (!runningInTauri()) {
      wfRunListeners.add(handler);
      return () => {
        wfRunListeners.delete(handler);
      };
    }
    const { listen } = await import("@tauri-apps/api/event");
    const u1 = await listen<{ run_id: string; workflow_name: string; started_at: number }>(
      "workflow_run_started",
      (ev) =>
        handler({
          type: "started",
          run_id: ev.payload.run_id,
          workflow_name: ev.payload.workflow_name,
          started_at: ev.payload.started_at,
        }),
    );
    const u2 = await listen<RunResult>("workflow_run_finished", (ev) =>
      handler({ type: "finished", result: ev.payload }),
    );
    const u3 = await listen<{ run_id: string; error: string }>(
      "workflow_run_errored",
      (ev) => handler({ type: "errored", run_id: ev.payload.run_id, error: ev.payload.error }),
    );
    return () => {
      u1();
      u2();
      u3();
    };
  },

  /**
   * Subscribe to skill hot-reload events (P30 WS2). The real `skills_reloaded`
   * event carries the latest `ScanReport`; we map it to `ModuleStatus` and hand
   * it to the handler so the UI can refresh both the list and the banner. Mock
   * mode is a no-op (returns an unlisten fn).
   */
  async subscribeSkillsReloaded(
    handler: (status: ModuleStatus) => void,
  ): Promise<() => void> {
    if (!runningInTauri()) return () => {};
    const { listen } = await import("@tauri-apps/api/event");
    const un = await listen<{
      skills: unknown[];
      issues: ModuleIssue[];
      scanned_dirs: number;
    }>("skills_reloaded", (ev) => {
      handler({
        skills: ev.payload.skills.map(mapRustSkill),
        issues: ev.payload.issues,
        scanned_dirs: ev.payload.scanned_dirs,
      });
    });
    return un;
  },

  /**
   * Subscribe to workflow-directory change events (P30 WS2). Real mode listens
   * to `workflows_changed`; the UI re-fetches the workflow list on fire. Mock
   * mode is a no-op.
   */
  async subscribeWorkflowsChanged(handler: () => void): Promise<() => void> {
    if (!runningInTauri()) return () => {};
    const { listen } = await import("@tauri-apps/api/event");
    const un = await listen("workflows_changed", () => handler());
    return un;
  },

  /**
   * List installed theme packages + any load issues (P31 · A3). Real mode calls
   * `invoke("list_themes")`; mock returns a single sample package so the picker
   * is demonstrable in the sandbox.
   */
  async listThemes(): Promise<ThemeListResult> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<ThemeListResult>("list_themes");
    }
    await sleep(60);
    return {
      themes: [
        {
          name: "ember",
          author: "caspian",
          version: "1.0.0",
          active: false,
        } as ThemeMeta,
      ],
      issues: [],
    };
  },

  /** Fetch a theme package's CSS variable overrides by name (P31). */
  async getThemeCss(name: string): Promise<string> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<string>("get_theme_css", { name });
    }
    await sleep(40);
    return MOCK_THEME_CSS;
  },

  /** Currently-active theme name, or null for built-in dark/light (P31). */
  async getActiveTheme(): Promise<string | null> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return (await invoke<string | null>("get_active_theme")) ?? null;
    }
    return null;
  },

  /**
   * Activate a theme and return its CSS for DOM injection (P31). The real
   * command also persists the selection and broadcasts `theme_changed` so other
   * windows update; the caller injects the returned `css` (see `lib/theme.ts`).
   */
  async applyTheme(name: string): Promise<string> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<string>("apply_theme", { name });
    }
    await sleep(40);
    return MOCK_THEME_CSS;
  },

  /**
   * Subscribe to theme changes (P31 WS2-style hot reload). Real mode listens to
   * `theme_changed` (carries `{ name, css }`); mock is a no-op.
   */
  async subscribeThemeChanged(
    handler: (e: ThemeChanged) => void,
  ): Promise<() => void> {
    if (!runningInTauri()) return () => {};
    const { listen } = await import("@tauri-apps/api/event");
    const un = await listen<ThemeChanged>("theme_changed", (ev) =>
      handler(ev.payload),
    );
    return un;
  },

  /**
   * Export the local state to a `.caspian` bundle directory (P36). Real mode
   * calls `invoke("export_bundle", { dest })`; mock resolves with a placeholder
   * manifest JSON so the UI flow is exercisable in the sandbox.
   */
  async exportBundle(dest: string): Promise<string> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<string>("export_bundle", { dest });
    }
    await sleep(60);
    return JSON.stringify({ format: "caspian-bundle", version: 1, items: [] });
  },

  /**
   * Import a `.caspian` bundle from `src` into local state (P36). `policy` is
   * "skip" (default) / "overwrite" / "rename". Returns a JSON `ImportReport`.
   */
  async importBundle(
    src: string,
    policy: "skip" | "overwrite" | "rename" = "skip",
  ): Promise<string> {
    if (runningInTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<string>("import_bundle", { src, policy });
    }
    await sleep(60);
    return JSON.stringify({ imported: ["(mock) demo_skill"], skipped: [], failed: [] });
  },
};

export function useCaspian() {
  return caspian;
}
