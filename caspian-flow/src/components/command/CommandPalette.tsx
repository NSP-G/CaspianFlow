import { Command } from "cmdk";
import { useNavigate } from "react-router-dom";
import { Plus, Settings, MessageSquare, Boxes, Library, Workflow } from "lucide-react";

/**
 * Cmd/Ctrl+K command palette — P25 skeleton (§四.6). Filters as you type; the
 * "设置" item jumps to the settings placeholder (验收 #6). Real navigation
 * targets (sessions, skills, agents) expand in later stages.
 */
export function CommandPalette({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
}) {
  const navigate = useNavigate();

  if (!open) return null;

  const go = (path: string) => {
    navigate(path);
    onOpenChange(false);
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 pt-[14vh]"
      onClick={() => onOpenChange(false)}
    >
      <div
        className="w-[520px] overflow-hidden rounded border border-border bg-card text-card-foreground"
        onClick={(e) => e.stopPropagation()}
      >
        <Command
          label="命令面板"
          className="flex flex-col"
          // cmdk filters by the input value; no animation, flat surface.
          shouldFilter
        >
          <Command.Input
            autoFocus
            placeholder="输入命令或搜索…（设置 / 新对话）"
            className="h-9 w-full border-b border-border bg-transparent px-3 text-[13px] text-foreground outline-none placeholder:text-muted-foreground"
          />
          <Command.List className="max-h-72 overflow-y-auto p-1">
            <Command.Empty className="px-3 py-3 text-[12px] text-muted-foreground">
              无匹配命令
            </Command.Empty>
            <Command.Group
              heading="导航"
              className="px-1 py-1 text-[11px] font-medium text-muted-foreground [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1"
            >
              <Command.Item
                value="新对话 new conversation"
                onSelect={() => go("/")}
                className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-[13px] text-foreground data-[selected=true]:bg-muted"
              >
                <Plus size={14} className="opacity-70" />
                新对话
              </Command.Item>
              <Command.Item
                value="设置 settings"
                onSelect={() => go("/settings")}
                className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-[13px] text-foreground data-[selected=true]:bg-muted"
              >
                <Settings size={14} className="opacity-70" />
                设置
              </Command.Item>
              <Command.Item
                value="会话 chat"
                onSelect={() => go("/")}
                className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-[13px] text-foreground data-[selected=true]:bg-muted"
              >
                <MessageSquare size={14} className="opacity-70" />
                返回会话
              </Command.Item>
              <Command.Item
                value="技能 skills marketplace"
                onSelect={() => go("/skills")}
                className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-[13px] text-foreground data-[selected=true]:bg-muted"
              >
                <Boxes size={14} className="opacity-70" />
                技能市场
              </Command.Item>
              <Command.Item
                value="知识库 knowledge"
                onSelect={() => go("/knowledge")}
                className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-[13px] text-foreground data-[selected=true]:bg-muted"
              >
                <Library size={14} className="opacity-70" />
                知识库
              </Command.Item>
              <Command.Item
                value="工作流 workflow canvas"
                onSelect={() => go("/workflows")}
                className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-[13px] text-foreground data-[selected=true]:bg-muted"
              >
                <Workflow size={14} className="opacity-70" />
                工作流
              </Command.Item>
            </Command.Group>
          </Command.List>
        </Command>
      </div>
    </div>
  );
}
