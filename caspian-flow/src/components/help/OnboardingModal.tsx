import { useState } from "react";
import { Button } from "@/components/ui/button";
import { HardDrive, MessageSquare, Keyboard } from "lucide-react";
import { useAppStore } from "@/stores/useAppStore";

const STEPS = [
  {
    icon: <HardDrive size={28} />,
    title: "你的数据在本地",
    body: "CaspianFlow 是本地优先应用：对话、技能、知识库与记忆全部保存在本机 ~/.caspian/，不上传云端，可离线运行。想迁移时，在「设置 → 数据导入/导出」生成一个 .caspian 包即可。",
  },
  {
    icon: <MessageSquare size={28} />,
    title: "用对话驱动一切",
    body: "在输入框描述任务，智能体会自动选择合适的 17 个内置技能（读文件、抓网页、跑 Python 等）完成任务。复杂流程可沉淀为「工作流」一键复用。",
  },
  {
    icon: <Keyboard size={28} />,
    title: "提速两个快捷键",
    body: "按 F1 随时呼出帮助面板（不离开当前页面）；按 Cmd/Ctrl + K 打开命令面板，几乎任何操作都能从那里触发。更多见帮助中心的「键盘快捷键」。",
  },
];

/**
 * First-run 3-step onboarding guide. Shown only when the user has not
 * dismissed it before (hasSeenOnboarding === false in useAppStore).
 */
export function OnboardingModal() {
  const [step, setStep] = useState(0);
  const setHasSeenOnboarding = useAppStore((s) => s.setHasSeenOnboarding);
  const total = STEPS.length;
  const current = STEPS[step];
  const isLast = step === total - 1;

  const finish = () => setHasSeenOnboarding(true);

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/50 p-4">
      <div className="w-full max-w-md rounded-2xl border border-border bg-background p-6 shadow-2xl">
        <div className="mb-4 flex items-center gap-3">
          <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-muted text-accent">
            {current.icon}
          </div>
          <div>
            <h2 className="text-base font-semibold">{current.title}</h2>
            <p className="text-xs text-muted-foreground">
              快速上手 · 第 {step + 1} / {total} 步
            </p>
          </div>
        </div>

        <p className="min-h-[72px] text-sm leading-relaxed text-muted-foreground">
          {current.body}
        </p>

        <div className="mt-2 flex gap-1.5">
          {STEPS.map((_, i) => (
            <span
              key={i}
              className={
                "h-1.5 flex-1 rounded-full transition-colors " +
                (i <= step ? "bg-accent" : "bg-muted")
              }
            />
          ))}
        </div>

        <div className="mt-5 flex items-center justify-between">
          <button
            type="button"
            onClick={finish}
            className="text-xs text-muted-foreground hover:text-foreground"
          >
            跳过引导
          </button>
          {isLast ? (
            <Button onClick={finish}>开始使用</Button>
          ) : (
            <Button onClick={() => setStep((s) => s + 1)}>下一步</Button>
          )}
        </div>
      </div>
    </div>
  );
}
