import { Minus, Square, X } from "lucide-react";
import { useAppStore } from "@/stores/useAppStore";
import { caspian } from "@/hooks/useCaspian";
import { cn } from "@/lib/utils";

/**
 * Custom title bar (P25 §四.1 / D2). No OS decorations; the whole bar is a
 * drag region. Window controls call the Tauri window API when running inside
 * the webview, and are inert in mock/preview.
 */

async function windowOp(op: "minimize" | "toggleMaximize" | "close") {
  if (!caspian.isTauri) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const w = getCurrentWindow();
  if (op === "minimize") await w.minimize();
  else if (op === "toggleMaximize") await w.toggleMaximize();
  else await w.close();
}

function CtrlButton({
  label,
  onClick,
  children,
  danger,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={cn(
        "flex h-7 w-9 items-center justify-center text-muted-foreground transition-colors hover:bg-muted",
        danger && "hover:bg-red-600 hover:text-white",
      )}
    >
      {children}
    </button>
  );
}

export function TitleBar() {
  const theme = useAppStore((s) => s.theme);

  return (
    <header
      data-tauri-drag-region
      className="flex h-9 shrink-0 items-center justify-between border-b border-border bg-background pl-3 pr-0 select-none"
    >
      <div data-tauri-drag-region className="flex items-center gap-2">
        <span className="text-[13px] font-semibold tracking-tight text-foreground">
          CaspianFlow
        </span>
        <span className="rounded-sm bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
          {theme === "dark" ? "暗色" : "浅色"}
        </span>
      </div>

      <div className="flex items-center">
        <CtrlButton label="最小化" onClick={() => void windowOp("minimize")}>
          <Minus size={14} />
        </CtrlButton>
        <CtrlButton label="最大化" onClick={() => void windowOp("toggleMaximize")}>
          <Square size={12} />
        </CtrlButton>
        <CtrlButton label="关闭" danger onClick={() => void windowOp("close")}>
          <X size={14} />
        </CtrlButton>
      </div>
    </header>
  );
}
