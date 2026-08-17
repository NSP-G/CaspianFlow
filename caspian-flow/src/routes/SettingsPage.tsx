import { useEffect, useState } from "react";
import { useAppStore } from "@/stores/useAppStore";
import { useCaspian } from "@/hooks/useCaspian";
import { applyThemeDom } from "@/lib/theme";
import { Button } from "@/components/ui/button";
import { Moon, Sun, FileText, Palette } from "lucide-react";
import type { ThemeIssue, ThemeListResult } from "@/types/theme";

/**
 * Settings page (P25 placeholder → P31 theme library).
 *
 * - Built-in dark/light toggle (P25 §十.3).
 * - Theme packages: list installed `~/.caspian/themes/*` packages, apply one by
 *   injecting its CSS overrides (`lib/theme.ts`), revert to built-in via
 *   "恢复默认主题". Broken packages surface as issues (§3 resilience: UI tells
 *   the user exactly what is missing).
 */
export function SettingsPage() {
  const theme = useAppStore((s) => s.theme);
  const toggleTheme = useAppStore((s) => s.toggleTheme);
  const customTheme = useAppStore((s) => s.customTheme);
  const applyCustomTheme = useAppStore((s) => s.applyCustomTheme);
  const clearCustomTheme = useAppStore((s) => s.clearCustomTheme);
  const caspian = useCaspian();

  const [themes, setThemes] = useState<ThemeListResult>({ themes: [], issues: [] });

  useEffect(() => {
    void caspian.listThemes().then(setThemes);
    // Reflect theme changes broadcast from other windows / disk watcher.
    void caspian.subscribeThemeChanged(() => {
      void caspian.listThemes().then(setThemes);
    });
  }, [caspian]);

  const applyPkg = async (name: string) => {
    const css = await caspian.applyTheme(name);
    applyCustomTheme(name, css);
    applyThemeDom();
  };

  const resetDefault = () => {
    clearCustomTheme();
    applyThemeDom();
  };

  // --- Data import / export (P36) ---
  const [bundlePath, setBundlePath] = useState("");
  const [bundleReport, setBundleReport] = useState<{
    imported?: string[];
    skipped?: string[];
    failed?: string[];
  } | null>(null);
  const [bundleBusy, setBundleBusy] = useState(false);

  const doExport = async () => {
    if (!bundlePath.trim()) return;
    setBundleBusy(true);
    try {
      const manifest = await caspian.exportBundle(bundlePath.trim());
      setBundleReport(JSON.parse(manifest));
    } finally {
      setBundleBusy(false);
    }
  };

  const doImport = async (policy: "skip" | "overwrite" | "rename") => {
    if (!bundlePath.trim()) return;
    setBundleBusy(true);
    try {
      const report = await caspian.importBundle(bundlePath.trim(), policy);
      setBundleReport(JSON.parse(report));
    } finally {
      setBundleBusy(false);
    }
  };

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-2xl space-y-6 p-6">
        <h1 className="text-base font-semibold text-foreground">设置</h1>

        <section className="space-y-2">
          <h2 className="text-[13px] font-medium text-muted-foreground">外观</h2>
          <div className="flex items-center gap-3">
            <Button variant="outline" onClick={toggleTheme} className="gap-2">
              {theme === "dark" ? <Sun size={15} /> : <Moon size={15} />}
              {theme === "dark" ? "切换到浅色" : "切换到暗色"}
            </Button>
            <span className="text-[12px] text-muted-foreground">
              当前：{theme === "dark" ? "暗色" : "浅色"}
              {customTheme ? ` · 主题包「${customTheme}」` : ""}
            </span>
          </div>
        </section>

        <section className="space-y-2">
          <h2 className="flex items-center gap-1.5 text-[13px] font-medium text-muted-foreground">
            <Palette size={14} /> 主题包
          </h2>
          <p className="text-[12px] text-muted-foreground">
            安装主题：将主题目录放入 <code>~/.caspian/themes/&lt;名称&gt;/</code>（含
            <code>manifest.yaml</code> 与 <code>theme.css</code>）。
          </p>

          <ul className="flex flex-col gap-1">
            {themes.themes.map((t) => {
              const active = customTheme === t.name;
              return (
                <li
                  key={t.name}
                  className="flex items-center gap-2 rounded border border-border px-2.5 py-1.5 text-[13px]"
                >
                  <Palette size={14} className="opacity-70" />
                  <span className="font-medium text-foreground">{t.name}</span>
                  <span className="text-[11px] text-muted-foreground">
                    {t.author ? `${t.author} · ` : ""}v{t.version}
                  </span>
                  <span className="ml-auto flex items-center gap-2">
                    {active && (
                      <span className="text-[11px] text-accent">已应用</span>
                    )}
                    <Button
                      variant={active ? "default" : "outline"}
                      size="sm"
                      onClick={() => void applyPkg(t.name)}
                    >
                      {active ? "保持" : "应用"}
                    </Button>
                  </span>
                </li>
              );
            })}
            {themes.themes.length === 0 && (
              <li className="rounded border border-dashed border-border px-2.5 py-1.5 text-[12px] text-muted-foreground">
                暂无已安装的主题包。
              </li>
            )}
          </ul>

          {customTheme && (
            <Button variant="ghost" size="sm" onClick={resetDefault} className="gap-1">
              <Sun size={13} /> 恢复默认主题
            </Button>
          )}

          {themes.issues.length > 0 && (
            <ul className="flex flex-col gap-1 pt-1">
              {themes.issues.map((iss: ThemeIssue, i) => (
                <li
                  key={i}
                  className="flex items-center gap-2 rounded border border-border px-2.5 py-1.5 text-[12px] text-muted-foreground"
                >
                  <FileText size={13} className="opacity-70" />
                  <span className="truncate">
                    {iss.name ?? iss.path}：{iss.reason}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="space-y-2">
          <h2 className="text-[13px] font-medium text-muted-foreground">
            本地文件（P28 完善）
          </h2>
          <ul className="flex flex-col gap-1">
            {["SOUL.md", "MEMORY.md", "USER.md"].map((f) => (
              <li
                key={f}
                className="flex items-center gap-2 rounded border border-border px-2.5 py-1.5 text-[13px] text-muted-foreground"
              >
                <FileText size={14} className="opacity-70" />
                <span className="truncate">{f}</span>
                <span className="ml-auto text-[11px] text-muted-foreground">
                  占位
                </span>
              </li>
            ))}
          </ul>
        </section>

        <section className="space-y-2">
          <h2 className="text-[13px] font-medium text-muted-foreground">
            数据导入 / 导出（P36 · .caspian 包）
          </h2>
          <p className="text-[12px] text-muted-foreground">
            将技能、配置、会话、知识整包导出为一个 <code>.caspian</code> 目录；导入时
            按策略处理冲突。
          </p>
          <input
            value={bundlePath}
            onChange={(e) => setBundlePath(e.target.value)}
            placeholder="导出目标目录 或 导入源目录 的绝对路径"
            className="w-full rounded border border-border bg-transparent px-2.5 py-1.5 text-[13px] text-foreground outline-none focus:border-accent"
          />
          <div className="flex flex-wrap items-center gap-2">
            <Button size="sm" variant="outline" disabled={bundleBusy} onClick={doExport}>
              导出
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={bundleBusy}
              onClick={() => doImport("skip")}
            >
              导入（跳过冲突）
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={bundleBusy}
              onClick={() => doImport("overwrite")}
            >
              导入（覆盖）
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={bundleBusy}
              onClick={() => doImport("rename")}
            >
              导入（重命名）
            </Button>
          </div>
          {bundleReport && (
            <div className="flex flex-col gap-1 rounded border border-border px-2.5 py-2 text-[12px] text-muted-foreground">
              <span>已导入：{bundleReport.imported?.length ?? 0}</span>
              <span>已跳过：{bundleReport.skipped?.length ?? 0}</span>
              <span>失败：{bundleReport.failed?.length ?? 0}</span>
              {bundleReport.failed && bundleReport.failed.length > 0 && (
                <ul className="list-disc pl-4 text-[11px]">
                  {bundleReport.failed.map((f, i) => (
                    <li key={i} className="truncate">
                      {f}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
