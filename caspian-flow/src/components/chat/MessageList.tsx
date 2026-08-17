import { useEffect, useRef } from "react";
import type { ChatMessage } from "@/types/chat";
import { MessageBubble } from "./MessageBubble";

/**
 * Scrollable message stream. Auto-scrolls to bottom as tokens arrive; the
 * agent bubble pre-allocates height so streaming doesn't shift the layout (§五).
 */
export function MessageList({ messages }: { messages: ChatMessage[] }) {
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [messages]);

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-3xl flex-col gap-3 px-4 py-4">
        {messages.length === 0 && (
          <div className="flex h-full items-center justify-center text-[13px] text-muted-foreground">
            发送一条消息开始对话。
          </div>
        )}
        {messages.map((m) => (
          <MessageBubble key={m.id} message={m} />
        ))}
        <div ref={endRef} />
      </div>
    </div>
  );
}
