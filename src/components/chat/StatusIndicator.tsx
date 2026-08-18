import { useEffect, useState } from "react";
import type { AgentStatus } from "@/types/chat";
import { AGENT_STATUS_LABEL, THINKING_PHRASES } from "@/lib/constants";

/**
 * Agent status indicator (P25 §四.4 / §五). Text + structure only — no spinner,
 * no color pulse. THINKING rotates its phrase every 3s locally.
 */
export function StatusIndicator({ status }: { status: AgentStatus }) {
  const [tick, setTick] = useState(0);

  useEffect(() => {
    if (status !== "THINKING") return;
    const t = setInterval(() => setTick((n) => n + 1), 3000);
    return () => clearInterval(t);
  }, [status]);

  const label =
    status === "THINKING"
      ? THINKING_PHRASES[tick % THINKING_PHRASES.length]
      : AGENT_STATUS_LABEL[status];

  return (
    <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
      <span className="h-1.5 w-1.5 rounded-full bg-accent" aria-hidden />
      <span className="tabular-nums">{label}</span>
    </div>
  );
}
