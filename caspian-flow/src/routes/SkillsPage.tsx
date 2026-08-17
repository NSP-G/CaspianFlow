import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Search, Boxes, ChevronRight, RefreshCw } from "lucide-react";
import { useCaspian } from "@/hooks/useCaspian";
import { Card } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { ModuleResilienceBanner } from "@/components/ModuleResilienceBanner";
import {
  SKILL_CATEGORIES,
  type ModuleStatus,
  type Skill,
  type SkillCategory,
} from "@/types/skills";

/**
 * Skill marketplace (P26 §二 页面一). Mock data via `useCaspian.listSkills`.
 *  - grid of flat cards (name / description / category tag / enable switch)
 *  - live search (name + description)
 *  - category filter chips
 *  - click a card to expand schema / permissions / triggers
 */
export function SkillsPage() {
  const caspian = useCaspian();
  const [skills, setSkills] = useState<Skill[]>([]);
  const [moduleStatus, setModuleStatus] = useState<ModuleStatus>({
    skills: [],
    issues: [],
    scanned_dirs: 0,
  });
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<SkillCategory | "all">("all");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [reloading, setReloading] = useState(false);

  useEffect(() => {
    void caspian.listSkills().then(setSkills);
    void caspian.getModuleStatus().then(setModuleStatus);
  }, [caspian]);

  // Hot-reload: refresh both the list and the banner when skills change on
  // disk (P30 WS2). Listens to the `skills_reloaded` event fired by the Rust
  // DirWatcher → SkillManager::reload path.
  useEffect(() => {
    let un: (() => void) | undefined;
    void caspian
      .subscribeSkillsReloaded((status) => {
        setSkills(status.skills);
        setModuleStatus(status);
      })
      .then((u) => {
        un = u;
      });
    return () => {
      un?.();
    };
  }, [caspian]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return skills.filter((s) => {
      const matchCat = category === "all" || s.category === category;
      const matchQ =
        q === "" ||
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q);
      return matchCat && matchQ;
    });
  }, [skills, query, category]);

  const toggle = async (id: string) => {
    const next = await caspian.toggleSkill(id);
    setSkills(next);
  };

  const reload = async () => {
    setReloading(true);
    try {
      const next = await caspian.reloadSkills();
      setSkills(next);
      setModuleStatus(await caspian.getModuleStatus());
    } finally {
      setReloading(false);
    }
  };

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-4xl space-y-4 p-6">
        {moduleStatus.issues.length > 0 && (
          <ModuleResilienceBanner issues={moduleStatus.issues} />
        )}
        <header className="flex flex-col gap-3">
          <div className="flex items-center gap-2">
            <Boxes size={18} className="text-accent" />
          <h1 className="text-base font-semibold text-foreground">技能市场</h1>
          <span className="text-[12px] text-muted-foreground">
            {filtered.length} / {skills.length}
          </span>
          <button
            type="button"
            onClick={() => void reload()}
            disabled={reloading}
            className="ml-auto flex items-center gap-1 rounded border border-border px-2 py-1 text-[12px] text-muted-foreground transition-colors hover:text-foreground disabled:opacity-50"
          >
            <RefreshCw size={13} className={reloading ? "animate-spin" : ""} />
            刷新
          </button>
        </div>

          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            <div className="relative flex-1">
              <Search
                size={14}
                className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground"
              />
              <Input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="搜索技能名称或描述…"
                className="pl-8"
              />
            </div>
            <div className="flex flex-wrap gap-1">
              <FilterChip
                active={category === "all"}
                onClick={() => setCategory("all")}
              >
                全部
              </FilterChip>
              {SKILL_CATEGORIES.map((c) => (
                <FilterChip
                  key={c.id}
                  active={category === c.id}
                  onClick={() => setCategory(c.id)}
                >
                  {c.label}
                </FilterChip>
              ))}
            </div>
          </div>
        </header>

        {filtered.length === 0 ? (
          <p className="py-10 text-center text-[13px] text-muted-foreground">
            没有匹配的技能
          </p>
        ) : (
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            {filtered.map((s) => (
              <SkillCard
                key={s.id}
                skill={s}
                expanded={expandedId === s.id}
                onToggle={() => void toggle(s.id)}
                onExpand={() =>
                  setExpandedId((id) => (id === s.id ? null : s.id))
                }
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function FilterChip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded border px-2 py-1 text-[12px] transition-colors",
        active
          ? "border-accent bg-accent text-accent-foreground"
          : "border-border bg-card text-muted-foreground hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

function SkillCard({
  skill,
  expanded,
  onToggle,
  onExpand,
}: {
  skill: Skill;
  expanded: boolean;
  onToggle: () => void;
  onExpand: () => void;
}) {
  const catLabel =
    SKILL_CATEGORIES.find((c) => c.id === skill.category)?.label ?? skill.category;

  return (
    <Card className="flex flex-col">
      <button
        type="button"
        onClick={onExpand}
        className="flex items-start gap-2 p-3 text-left"
      >
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="font-mono text-[13px] text-foreground">
              {skill.name}
            </span>
            <span className="rounded border border-border px-1.5 py-0.5 text-[11px] text-muted-foreground">
              {catLabel}
            </span>
          </div>
          <p className="mt-1 text-[12px] leading-snug text-muted-foreground">
            {skill.description}
          </p>
        </div>
        <ChevronRight
          size={15}
          className={cn(
            "mt-0.5 shrink-0 text-muted-foreground transition-transform",
            expanded && "rotate-90",
          )}
        />
      </button>

      <div
        className="flex items-center justify-between border-t border-border px-3 py-2"
        onClick={(e) => e.stopPropagation()}
      >
        <span
          className={cn(
            "text-[12px]",
            skill.enabled ? "text-accent" : "text-muted-foreground",
          )}
        >
          {skill.enabled ? "已启用" : "已禁用"}
        </span>
        <Switch
          checked={skill.enabled}
          onCheckedChange={() => onToggle()}
          aria-label={`切换 ${skill.name}`}
        />
      </div>

      {expanded && (
        <div className="space-y-3 border-t border-border p-3 text-[12px]">
          <DetailBlock label="Schema">
            <code className="block whitespace-pre-wrap font-mono text-[11px] text-foreground">
              {skill.schema}
            </code>
          </DetailBlock>
          <DetailBlock label="权限">
            <ul className="flex flex-col gap-0.5">
              {skill.permissions.map((p) => (
                <li key={p} className="text-muted-foreground">
                  · {p}
                </li>
              ))}
            </ul>
          </DetailBlock>
          <DetailBlock label="触发短语">
            <ul className="flex flex-col gap-0.5">
              {skill.triggers.map((t) => (
                <li key={t} className="text-muted-foreground">
                  “{t}”
                </li>
              ))}
            </ul>
          </DetailBlock>
        </div>
      )}
    </Card>
  );
}

function DetailBlock({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div>
      <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </div>
      {children}
    </div>
  );
}
