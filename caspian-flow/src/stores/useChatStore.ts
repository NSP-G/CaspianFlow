import { create } from "zustand";
import type { AgentStatus, AgentStatusEvent, ChatMessage, Session, StreamChunk } from "@/types/chat";
import { caspian } from "@/hooks/useCaspian";
import { DATA_PATH_LABEL, MOCK_DATA_PATH } from "@/lib/constants";

let idCounter = 0;
function uid(prefix: string): string {
  idCounter += 1;
  return `${prefix}_${Date.now().toString(36)}_${idCounter}`;
}

interface ChatState {
  sessions: Session[];
  currentSessionId: string;
  messages: ChatMessage[];
  agentStatus: AgentStatus;
  dataPath: string;
  streamingMessageId: string | null;
  initialized: boolean;

  init: () => Promise<void>;
  newSession: () => void;
  setCurrentSession: (id: string) => void;
  send: (text: string) => Promise<void>;
}

export const useChatStore = create<ChatState>()((set, get) => ({
  sessions: [],
  currentSessionId: "s_demo",
  messages: [],
  agentStatus: "IDLE",
  dataPath: MOCK_DATA_PATH,
  streamingMessageId: null,
  initialized: false,

  init: async () => {
    if (get().initialized) return;
    const [sessions, dataPath] = await Promise.all([
      caspian.listSessions(),
      caspian.getDataPath(),
    ]);
    set({ sessions, dataPath, initialized: true });
    // In real (Tauri) mode, route events through the store; mock delivers via
    // send() callbacks, so this is a no-op there.
    void caspian.subscribe(
      (e: AgentStatusEvent) => applyStatus(e),
      (e: StreamChunk) => appendChunk(e),
    );
  },

  newSession: () => {
    const id = uid("s");
    const session: Session = { id, title: "新对话", updatedAt: Date.now() };
    set((s) => ({
      sessions: [session, ...s.sessions],
      currentSessionId: id,
      messages: [],
    }));
  },

  setCurrentSession: (id) => {
    set({ currentSessionId: id, messages: [] });
  },

  send: async (text: string) => {
    const trimmed = text.trim();
    if (!trimmed) return;
    const sessionId = get().currentSessionId;
    const userMsg: ChatMessage = {
      id: uid("m"),
      role: "user",
      content: trimmed,
      createdAt: Date.now(),
    };
    const agentId = uid("m");
    const agentMsg: ChatMessage = {
      id: agentId,
      role: "agent",
      content: "",
      createdAt: Date.now(),
      status: "THINKING",
      streaming: true,
    };
    set((s) => ({
      messages: [...s.messages, userMsg, agentMsg],
      streamingMessageId: agentId,
      agentStatus: "THINKING",
    }));

    await caspian.sendMessage(sessionId, trimmed, {
      onStatus: (e) => applyStatus(e),
      onChunk: (e) => appendChunk(e),
    });
  },
}));

/** Apply an agent-status event to the store + the active streaming message. */
function applyStatus(e: AgentStatusEvent) {
  const { streamingMessageId } = useChatStore.getState();
  useChatStore.setState((s) => ({
    agentStatus: e.status,
    messages: s.messages.map((m) =>
      m.id === streamingMessageId
        ? { ...m, status: e.status, streaming: e.status === "STREAMING_ANSWER" }
        : m,
    ),
  }));
}

/** Append a streamed chunk to the active agent message. */
function appendChunk(e: StreamChunk) {
  const { streamingMessageId } = useChatStore.getState();
  if (!streamingMessageId) return;
  useChatStore.setState((s) => ({
    messages: s.messages.map((m) =>
      m.id === streamingMessageId ? { ...m, content: m.content + e.chunk } : m,
    ),
  }));
}

// Re-export so components importing from the store get the sidebar label too.
export { DATA_PATH_LABEL };
