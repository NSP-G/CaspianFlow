// Workflow list view (P27 验收 #1): shows saved workflows with name, modified
// time, step count, and a delete action. "新建工作流" seeds a blank draft and
// opens the editor.

import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Plus, Trash2, FileCode2, Workflow as WorkflowIcon } from "lucide-react";
import { useCaspian } from "@/hooks/useCaspian";
import { blankDoc } from "@/lib/workflow";
import type { WorkflowListEntry } from "@/types/workflow";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

function relativeTime(ts: number): string {
  if (!ts) return "—";
  const diff = Date.now() - ts * 1000;
  const s = Math.floor(diff / 1000);
  if (s < 60) return "刚刚";
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} 分钟前`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h} 小时前`;
  const d = Math.floor(h / 24);
  return `${d} 天前`;
}

export function WorkflowsPage() {
  const navigate = useNavigate();
  const capi = useCaspian();
  const [entries, setEntries] = useState<WorkflowListEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [confirm, setConfirm] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    const list = await capi.listWorkflows();
    setEntries(list);
    setLoading(false);
  }, [capi]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Hot-reload: re-fetch the list when the workflows directory changes on
  // disk (P30 WS2). The Rust DirWatcher fires `workflows_changed`.
  useEffect(() => {
    let un: (() => void) | undefined;
    void capi.subscribeWorkflowsChanged(() => void refresh()).then((u) => {
      un = u;
    });
    return () => {
      un?.();
    };
  }, [capi, refresh]);

  const newWorkflow = async () => {
    const name = `wf_${Date.now()}`;
    await capi.saveWorkflowDraft(name, blankDoc(name));
    navigate(`/workflows/${name}`);
  };

  const remove = async (name: string) => {
    await capi.deleteWorkflow(name);
    setConfirm(null);
    void refresh();
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-2 border-b border-border px-4 py-3">
        <WorkflowIcon size={16} className="text-accent" />
        <h1 className="text-[15px] font-medium">工作流</h1>
        <Button variant="primary" size="sm" className="ml-auto" onClick={newWorkflow}>
          <Plus size={14} />
          新建工作流
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {loading ? (
          <p className="text-[13px] text-muted-foreground">加载中…</p>
        ) : entries.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-2 py-16 text-center">
            <FileCode2 size={28} className="text-muted-foreground" />
            <p className="text-[13px] text-muted-foreground">还没有工作流</p>
            <Button variant="outline" size="sm" onClick={newWorkflow}>
              <Plus size={14} />
              创建第一个
            </Button>
          </div>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {entries.map((e) => (
              <li
                key={e.name}
                className={cn(
                  "group flex items-center gap-3 rounded border border-border bg-card px-3 py-2.5 transition-colors hover:bg-muted",
                )}
              >
                <button
                  type="button"
                  onClick={() => navigate(`/workflows/${e.name}`)}
                  className="flex min-w-0 flex-1 items-center gap-3 text-left"
                >
                  <FileCode2 size={16} className="shrink-0 text-accent" />
                  <span className="min-w-0">
                    <span className="block truncate text-[13px] text-foreground">
                      {e.display_name || e.name}
                    </span>
                    <span className="block truncate text-[11px] text-muted-foreground">
                      {e.name} · {e.step_count} 步 · {relativeTime(e.modified)}
                    </span>
                  </span>
                </button>
                {confirm === e.name ? (
                  <span className="flex items-center gap-1.5">
                    <span className="text-[11px] text-muted-foreground">确认删除？</span>
                    <Button variant="primary" size="sm" onClick={() => remove(e.name)}>
                      删除
                    </Button>
                    <Button variant="ghost" size="sm" onClick={() => setConfirm(null)}>
                      取消
                    </Button>
                  </span>
                ) : (
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label="删除工作流"
                    title="删除"
                    className="opacity-0 group-hover:opacity-100"
                    onClick={() => setConfirm(e.name)}
                  >
                    <Trash2 size={15} className="text-danger" />
                  </Button>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
