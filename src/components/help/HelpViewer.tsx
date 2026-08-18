import { useState } from "react";
import { Markdown } from "@/lib/markdown";

// Load every help markdown file at build time (Vite ?raw import).
// HelpViewer lives in src/components/help/, so three levels up reaches the
// project root where docs/help/ lives.
const rawModules = import.meta.glob("../../../docs/help/*.md", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

interface Topic {
  id: string;
  title: string;
}

// Order here defines the navigation order in both the page and the panel.
const TOPICS: Topic[] = [
  { id: "index", title: "帮助中心" },
  { id: "getting-started", title: "快速上手（3 步）" },
  { id: "skills", title: "内置技能" },
  { id: "workflows", title: "工作流" },
  { id: "keyboard-shortcuts", title: "键盘快捷键" },
  { id: "faq", title: "常见问题（FAQ）" },
];

function contentOf(id: string): string {
  const key = `../../docs/help/${id}.md`;
  return rawModules[key] ?? "# 文档缺失\n\n未能加载该帮助主题的内容。";
}

export function HelpViewer() {
  const [active, setActive] = useState<string>(TOPICS[0].id);
  const content = contentOf(active);

  return (
    <div className="flex min-h-0 flex-1">
      <nav className="w-48 shrink-0 overflow-y-auto border-r border-border p-3">
        <p className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          主题
        </p>
        <ul className="space-y-1">
          {TOPICS.map((t) => (
            <li key={t.id}>
              <button
                type="button"
                onClick={() => setActive(t.id)}
                className={
                  "w-full rounded px-2 py-1.5 text-left text-sm transition-colors " +
                  (active === t.id
                    ? "bg-muted font-medium text-foreground"
                    : "text-muted-foreground hover:bg-muted hover:text-foreground")
                }
              >
                {t.title}
              </button>
            </li>
          ))}
        </ul>
      </nav>
      <article className="min-w-0 flex-1 overflow-y-auto p-6">
        <Markdown content={content} />
      </article>
    </div>
  );
}
