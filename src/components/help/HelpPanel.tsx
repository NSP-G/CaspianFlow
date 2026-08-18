import { X } from "lucide-react";
import { HelpViewer } from "@/components/help/HelpViewer";

/**
 * Slide-over help panel triggered by F1 (see useHelp). Renders on top of the
 * current page without unmounting it, so closing returns to the same context.
 */
export function HelpPanel({ open, onClose }: { open: boolean; onClose: () => void }) {
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      <div
        className="absolute inset-0 bg-black/40"
        onClick={onClose}
        aria-hidden="true"
      />
      <aside className="relative flex h-full w-full max-w-3xl flex-col bg-background shadow-xl">
        <header className="flex items-center justify-between border-b border-border px-6 py-3">
          <h2 className="text-base font-semibold">帮助</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭帮助"
            className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X size={18} />
          </button>
        </header>
        <HelpViewer />
      </aside>
    </div>
  );
}
