import { useChatStore } from "@/stores/useChatStore";
import { MessageList } from "@/components/chat/MessageList";
import { ChatInput } from "@/components/chat/ChatInput";
import { StatusIndicator } from "@/components/chat/StatusIndicator";

/**
 * Main chat page (P25 §四.3). Message stream fills the space; the agent status
 * indicator + input sit at the bottom. Theme toggle is in the sidebar/title bar.
 */
export function ChatPage() {
  const messages = useChatStore((s) => s.messages);
  const agentStatus = useChatStore((s) => s.agentStatus);

  return (
    <div className="flex h-full flex-col">
      <MessageList messages={messages} />
      <div className="shrink-0 border-t border-border px-4 py-2.5">
        <div className="mx-auto max-w-3xl">
          <div className="mb-2">
            <StatusIndicator status={agentStatus} />
          </div>
          <ChatInput />
        </div>
      </div>
    </div>
  );
}
