import type { ChatMessage } from "@/types/chat";
import { cn } from "@/lib/utils";

/**
 * Single message bubble (P25 §四.3). User right-aligned, agent left-aligned.
 * No card-in-card, no shadow — a 1px border + 4px radius defines the surface.
 */
export function MessageBubble({ message }: { message: ChatMessage }) {
  const isUser = message.role === "user";

  if (isUser) {
    return (
      <div className="flex justify-end">
        <div className="max-w-[78%] rounded border border-border bg-muted px-3 py-2 text-[13px] leading-relaxed text-foreground">
          <p className="whitespace-pre-wrap break-words">{message.content}</p>
        </div>
      </div>
    );
  }

  // Agent: reserve height while streaming so layout doesn't jitter (§五).
  const showPlaceholder = message.streaming && message.content.length === 0;
  return (
    <div className="flex justify-start">
      <div
        className={cn(
          "max-w-[82%] rounded border border-border bg-card px-3 py-2 text-[13px] leading-relaxed text-card-foreground",
          message.streaming && "min-h-[2.5rem]",
        )}
      >
        {showPlaceholder ? (
          <p className="text-muted-foreground">思考中…</p>
        ) : (
          <p className="whitespace-pre-wrap break-words">{message.content}</p>
        )}
        {message.status === "ERROR" && message.error && (
          <p className="mt-1 text-[12px] text-red-500">{message.error}</p>
        )}
      </div>
    </div>
  );
}
