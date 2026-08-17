import { useEffect } from "react";
import { useNavigate, useLocation } from "react-router-dom";
import { Plus, MessageSquare, Settings, HardDrive, Boxes, Library, Workflow, HelpCircle } from "lucide-react";
import { useChatStore } from "@/stores/useChatStore";
import { useAppStore } from "@/stores/useAppStore";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { DATA_PATH_LABEL } from "@/lib/constants";

/**
 * Sidebar skeleton (P25 §四.2 / §六): conversation list (placeholder) + a
 * "new conversation" action + bottom local-first trust signal + settings entry.
 */
export function Sidebar({ collapsed }: { collapsed: boolean }) {
  const navigate = useNavigate();
  const location = useLocation();
  const sessions = useChatStore((s) => s.sessions);
  const currentSessionId = useChatStore((s) => s.currentSessionId);
  const newSession = useChatStore((s) => s.newSession);
  const setCurrentSession = useChatStore((s) => s.setCurrentSession);
  const dataPath = useChatStore((s) => s.dataPath);
  const init = useChatStore((s) => s.init);
  const sidebarCollapsed = useAppStore((s) => s.sidebarCollapsed);

  const navItem = (path: string) =>
    location.pathname === path
      ? "bg-muted text-foreground"
      : "text-muted-foreground hover:bg-muted hover:text-foreground";

  useEffect(() => {
    void init();
  }, [init]);

  if (collapsed || sidebarCollapsed) {
    return (
      <aside className="flex w-12 shrink-0 flex-col items-center gap-2 border-r border-border bg-background py-2">
        <Button
          variant="ghost"
          size="icon"
          aria-label="新对话"
          title="新对话"
          onClick={newSession}
        >
          <Plus size={16} />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label="设置"
          title="设置"
          onClick={() => navigate("/settings")}
        >
          <Settings size={16} />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label="技能"
          title="技能"
          onClick={() => navigate("/skills")}
        >
          <Boxes size={16} className={cn(location.pathname === "/skills" && "text-accent")} />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label="知识库"
          title="知识库"
          onClick={() => navigate("/knowledge")}
        >
          <Library size={16} className={cn(location.pathname === "/knowledge" && "text-accent")} />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label="工作流"
          title="工作流"
          onClick={() => navigate("/workflows")}
        >
          <Workflow size={16} className={cn(location.pathname.startsWith("/workflows") && "text-accent")} />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label="帮助"
          title="帮助 (F1)"
          onClick={() => navigate("/help")}
        >
          <HelpCircle size={16} className={cn(location.pathname === "/help" && "text-accent")} />
        </Button>
      </aside>
    );
  }

  return (
    <aside className="flex w-60 shrink-0 flex-col border-r border-border bg-background">
      <div className="p-2">
        <Button
          variant="outline"
          className="w-full justify-start gap-2"
          onClick={newSession}
        >
          <Plus size={15} />
          新对话
        </Button>
      </div>

      <nav className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
        <ul className="flex flex-col gap-0.5">
          {sessions.map((s) => (
            <li key={s.id}>
              <button
                type="button"
                onClick={() => setCurrentSession(s.id)}
                className={cn(
                  "flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[13px] transition-colors",
                  s.id === currentSessionId
                    ? "bg-muted text-foreground"
                    : "text-muted-foreground hover:bg-muted hover:text-foreground",
                )}
              >
                <MessageSquare size={14} className="shrink-0 opacity-70" />
                <span className="truncate">{s.title}</span>
              </button>
            </li>
          ))}
          {sessions.length === 0 && (
            <li className="px-2 py-1.5 text-[12px] text-muted-foreground">
              暂无会话
            </li>
          )}
        </ul>
      </nav>

      <div className="border-t border-border p-2">
        <div className="flex items-center gap-1.5 px-1 pb-2 text-[11px] text-muted-foreground">
          <HardDrive size={12} className="shrink-0" />
          <span className="truncate">{dataPath || DATA_PATH_LABEL}</span>
        </div>
        <div className="flex flex-col gap-0.5">
          <button
            type="button"
            onClick={() => navigate("/skills")}
            className={cn(
              "flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[13px] transition-colors",
              navItem("/skills"),
            )}
          >
            <Boxes size={14} className="shrink-0 opacity-70" />
            <span className="truncate">技能</span>
          </button>
          <button
            type="button"
            onClick={() => navigate("/knowledge")}
            className={cn(
              "flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[13px] transition-colors",
              navItem("/knowledge"),
            )}
          >
            <Library size={14} className="shrink-0 opacity-70" />
            <span className="truncate">知识库</span>
          </button>
          <button
            type="button"
            onClick={() => navigate("/workflows")}
            className={cn(
              "flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[13px] transition-colors",
              navItem("/workflows"),
            )}
          >
            <Workflow size={14} className="shrink-0 opacity-70" />
            <span className="truncate">工作流</span>
          </button>
          <Button
            variant="ghost"
            className="w-full justify-start gap-2 text-muted-foreground"
            onClick={() => navigate("/settings")}
          >
            <Settings size={15} />
            设置
          </Button>
          <Button
            variant="ghost"
            className={cn("w-full justify-start gap-2", navItem("/help"))}
            onClick={() => navigate("/help")}
          >
            <HelpCircle size={15} />
            帮助
          </Button>
        </div>
      </div>
    </aside>
  );
}
