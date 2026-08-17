import { HelpViewer } from "@/components/help/HelpViewer";

/**
 * /help route — the full-page built-in help browser.
 * The same content is also surfaced as a slide-over via HelpPanel (F1).
 */
export function HelpPage() {
  return (
    <div className="flex h-full flex-col">
      <header className="border-b border-border px-6 py-3">
        <h1 className="text-lg font-semibold">帮助</h1>
        <p className="text-xs text-muted-foreground">
          内置帮助文档 · 按 F1 可随时浮层呼出
        </p>
      </header>
      <HelpViewer />
    </div>
  );
}
