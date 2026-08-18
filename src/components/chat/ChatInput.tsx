import { useState, type KeyboardEvent } from "react";
import { Send } from "lucide-react";
import { useChatStore } from "@/stores/useChatStore";
import { Button } from "@/components/ui/button";

/**
 * Input box + send (P25 §四.3). Enter sends, Shift+Enter inserts a newline.
 * Disabled while the agent is producing an answer to avoid interleaving.
 */
export function ChatInput() {
  const [text, setText] = useState("");
  const send = useChatStore((s) => s.send);
  const busy = useChatStore(
    (s) => s.agentStatus === "THINKING" || s.agentStatus === "STREAMING_ANSWER",
  );

  const submit = () => {
    const value = text.trim();
    if (!value || busy) return;
    setText("");
    void send(value);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <div className="flex items-end gap-2">
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        rows={1}
        placeholder="给 CaspianFlow 发消息…（Enter 发送，Shift+Enter 换行）"
        className="max-h-40 min-h-[2rem] flex-1 resize-none rounded border border-input bg-transparent px-2.5 py-1.5 text-[13px] text-foreground outline-none placeholder:text-muted-foreground focus-visible:border-ring"
      />
      <Button onClick={submit} disabled={busy || !text.trim()} aria-label="发送">
        <Send size={14} />
        发送
      </Button>
    </div>
  );
}
