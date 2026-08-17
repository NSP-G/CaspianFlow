import { AlertTriangle } from "lucide-react";
import type { ModuleIssue, ModuleIssueKind } from "@/types/skills";

const KIND_LABEL: Record<ModuleIssueKind, string> = {
  missing_manifest: "缺少 skill.yaml",
  read_error: "读取失败",
  parse_error: "解析失败",
  validation_error: "校验失败",
};

/**
 * Non-blocking banner that surfaces modules which failed to load (P30 WS1 §3).
 *
 * This is the concrete delivery of the "UI 精确告知缺失" philosophy: instead of
 * a silently-dropped skill, the user sees *what* is missing, *where*, and *why*.
 * Renders nothing when there are no issues (so a healthy install stays clean).
 */
export function ModuleResilienceBanner({ issues }: { issues: ModuleIssue[] }) {
  if (!issues || issues.length === 0) return null;

  return (
    <div className="space-y-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3">
      <div className="flex items-center gap-2 text-[13px] font-medium text-amber-300">
        <AlertTriangle size={15} />
        <span>{issues.length} 个技能模块未加载</span>
      </div>
      <ul className="space-y-1.5">
        {issues.map((issue, i) => (
          <li
            key={i}
            className="flex flex-col gap-0.5 rounded border border-border bg-card px-2.5 py-1.5 text-[12px]"
          >
            <div className="flex items-center gap-2">
              <span className="rounded bg-amber-500/20 px-1.5 py-0.5 text-[11px] text-amber-300">
                {KIND_LABEL[issue.kind]}
              </span>
              {issue.skill_name && (
                <span className="font-mono text-foreground">{issue.skill_name}</span>
              )}
            </div>
            <code
              className="block truncate font-mono text-[11px] text-muted-foreground"
              title={issue.path}
            >
              {issue.path}
            </code>
            <span className="text-muted-foreground">{issue.reason}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
