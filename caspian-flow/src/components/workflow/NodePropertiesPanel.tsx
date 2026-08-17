// Node property panel (P29): edit a single step's config in a right-hand
// sidebar. Field names mirror P17 `WorkflowStep` (schema.rs:127) exactly so the
// edits round-trip through `save_workflow`'s JSON→YAML passthrough into a
// `Workflow::load`-able manifest (≠ P27 field names in the design doc — see
// P29_PRECHECK.md F1/F2/F3 for the reconciliation).

import { useState, type ReactNode } from "react";
import { Code2, List, Plus, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import {
  type StepNode,
  type StepNodeData,
  RETRY_MAX,
  RETRY_MIN,
  TIMEOUT_MAX,
  TIMEOUT_MIN,
  validateInputJson,
  validateRetry,
  validateTimeout,
} from "@/lib/workflow";

const TEXTAREA_CLS =
  "flex w-full rounded border border-input bg-transparent px-2.5 py-1 text-foreground outline-none placeholder:text-muted-foreground focus-visible:border-ring";

interface NodePropertiesPanelProps {
  node: StepNode;
  onChange: (patch: Partial<StepNodeData>) => void;
}

export function NodePropertiesPanel({ node, onChange }: NodePropertiesPanelProps) {
  const data = node.data;
  const [inputMode, setInputMode] = useState<"form" | "json">("form");
  const [jsonText, setJsonText] = useState("");
  const [jsonErr, setJsonErr] = useState<string | null>(null);

  const inputObj = (data.input ?? {}) as Record<string, unknown>;
  const inputEntries = Object.entries(inputObj);

  function commitInput(obj: Record<string, unknown>) {
    onChange({ input: obj });
  }
  function setInputValue(key: string, value: string) {
    commitInput({ ...inputObj, [key]: value });
  }
  function removeInput(key: string) {
    const next = { ...inputObj };
    delete next[key];
    commitInput(next);
  }
  function addInput(key: string, value: string) {
    if (!key.trim()) return;
    commitInput({ ...inputObj, [key]: value });
  }
  function switchToJson() {
    setJsonText(JSON.stringify(inputObj, null, 2));
    setJsonErr(null);
    setInputMode("json");
  }
  function onJsonChange(text: string) {
    setJsonText(text);
    const err = validateInputJson(text);
    setJsonErr(err);
    if (!err) {
      try {
        onChange({ input: JSON.parse(text) as Record<string, unknown> });
      } catch {
        /* already flagged by validateInputJson */
      }
    }
  }

  const timeoutErr = validateTimeout(data.timeout);
  const retryErr = validateRetry(data.retry_count);

  return (
    <aside className="flex w-[300px] shrink-0 flex-col border-l border-border bg-background">
      <div className="border-b border-border px-3 py-2">
        <div className="text-[12px] font-medium text-foreground">节点属性</div>
        <div className="mt-0.5 font-mono text-[11px] text-muted-foreground">步骤 · {node.id}</div>
      </div>

      <div className="min-h-0 flex-1 space-y-4 overflow-auto px-3 py-3">
        <Field label="技能 (skill)" hint="在画布节点上直接编辑">
          <Input value={data.skill} readOnly disabled className="font-mono" />
        </Field>

        <Field label="输入参数 (input)" hint="技能入参 · 模板表达式 ${variables.x} / ${steps.y.output}">
          <div className="mb-1.5 flex gap-1">
            <TabBtn active={inputMode === "form"} onClick={() => setInputMode("form")}>
              <List size={12} /> 表单
            </TabBtn>
            <TabBtn active={inputMode === "json"} onClick={switchToJson}>
              <Code2 size={12} /> JSON
            </TabBtn>
          </div>

          {inputMode === "form" ? (
            <div className="space-y-1.5">
              {inputEntries.length === 0 && (
                <p className="text-[11px] text-muted-foreground">暂无入参，用下方添加。</p>
              )}
              {inputEntries.map(([k, v]) => (
                <div key={k} className="flex items-center gap-1">
                  <span
                    className="w-24 shrink-0 truncate font-mono text-[11px] text-muted-foreground"
                    title={k}
                  >
                    {k}
                  </span>
                  <Input
                    value={String(v ?? "")}
                    onChange={(e) => setInputValue(k, e.target.value)}
                    className="h-7 flex-1 font-mono text-[11px]"
                  />
                  <button
                    type="button"
                    className="shrink-0 rounded p-1 text-muted-foreground hover:text-danger"
                    aria-label="删除入参"
                    onClick={() => removeInput(k)}
                  >
                    <Trash2 size={13} />
                  </button>
                </div>
              ))}
              <AddInputRow onAdd={addInput} />
            </div>
          ) : (
            <div className="space-y-1">
              <textarea
                className={cn(TEXTAREA_CLS, "h-32 font-mono text-[11px]")}
                value={jsonText}
                spellCheck={false}
                placeholder={'{\n  "path": "${variables.input_path}"\n}'}
                onChange={(e) => onJsonChange(e.target.value)}
              />
              {jsonErr && <p className="text-[11px] text-danger">{jsonErr}</p>}
            </div>
          )}
        </Field>

        <Field label="输出变量名 (output)" hint="后续以 ${steps.<id>.output} 引用">
          <Input
            value={data.output ?? ""}
            placeholder="例如 summary"
            onChange={(e) => onChange({ output: e.target.value || undefined })}
            className="font-mono text-[12px]"
          />
        </Field>

        <Field label="条件 (condition)" hint="为假则跳过本步骤">
          <textarea
            className={cn(TEXTAREA_CLS, "h-16 font-mono text-[11px]")}
            value={data.condition ?? ""}
            placeholder="${steps.a.output.n} > 1000"
            spellCheck={false}
            onChange={(e) => onChange({ condition: e.target.value || undefined })}
          />
        </Field>

        <Field label="超时 (timeout)" hint={`秒 · ${TIMEOUT_MIN}–${TIMEOUT_MAX}，留空用工作流默认`}>
          <Input
            type="number"
            min={TIMEOUT_MIN}
            max={TIMEOUT_MAX}
            value={data.timeout ?? ""}
            onChange={(e) => {
              const raw = e.target.value;
              onChange({ timeout: raw === "" ? undefined : Number(raw) });
            }}
            className="text-[12px]"
          />
          {timeoutErr && <p className="mt-1 text-[11px] text-danger">{timeoutErr}</p>}
        </Field>

        <Field label="重试 (retry_count)" hint={`次 · ${RETRY_MIN}–${RETRY_MAX}，留空用工作流配置`}>
          <Input
            type="number"
            min={RETRY_MIN}
            max={RETRY_MAX}
            value={data.retry_count ?? ""}
            onChange={(e) => {
              const raw = e.target.value;
              onChange({ retry_count: raw === "" ? undefined : Number(raw) });
            }}
            className="text-[12px]"
          />
          {retryErr && <p className="mt-1 text-[11px] text-danger">{retryErr}</p>}
        </Field>

        <p className="border-t border-border pt-2 text-[10px] leading-relaxed text-muted-foreground">
          字段名对应 P17 <code className="font-mono">WorkflowStep</code>。步骤级
          <code className="font-mono">on_error</code> 不在 P17 步骤结构中，错误策略由工作流级
          <code className="font-mono">error_handling</code> 控制。
        </p>
      </div>
    </aside>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div>
      <div className="mb-1 text-[12px] font-medium text-foreground">{label}</div>
      {hint && <div className="mb-1 text-[10px] text-muted-foreground">{hint}</div>}
      {children}
    </div>
  );
}

function TabBtn({
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
        "flex items-center gap-1 rounded border px-2 py-0.5 text-[11px]",
        active
          ? "border-accent text-accent"
          : "border-border text-muted-foreground hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

function AddInputRow({ onAdd }: { onAdd: (k: string, v: string) => void }) {
  const [k, setK] = useState("");
  const [v, setV] = useState("");
  return (
    <div className="flex items-center gap-1">
      <Input
        value={k}
        placeholder="键"
        onChange={(e) => setK(e.target.value)}
        className="h-7 w-24 font-mono text-[11px]"
      />
      <Input
        value={v}
        placeholder="值"
        onChange={(e) => setV(e.target.value)}
        className="h-7 flex-1 font-mono text-[11px]"
      />
      <button
        type="button"
        className="shrink-0 rounded border border-border p-1 text-muted-foreground hover:text-foreground"
        aria-label="添加入参"
        onClick={() => {
          onAdd(k, v);
          setK("");
          setV("");
        }}
      >
        <Plus size={13} />
      </button>
    </div>
  );
}
