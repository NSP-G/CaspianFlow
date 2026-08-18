/**
 * Domain types for the chat UI. Hand-written stand-ins for the ts-rs generated
 * types (P25 mock stage). When P21/P22 real interfaces land, these will be
 * replaced by `src/types/generated/*` from the Rust side.
 */

/** Agent lifecycle states (P25 §五). UI expresses these via text + structure,
 *  never via spinners or color pulse. */
export type AgentStatus =
  | "IDLE"
  | "THINKING"
  | "AWAITING_CONFIRMATION"
  | "EXECUTING_TOOL"
  | "STREAMING_ANSWER"
  | "UNCERTAIN"
  | "ERROR";

export type ChatRole = "user" | "agent";

export interface ChatMessage {
  id: string;
  role: ChatRole;
  content: string;
  /** Epoch ms. */
  createdAt: number;
  /** Present on agent messages while a tool runs / an answer streams. */
  status?: AgentStatus;
  /** True while tokens are still arriving (pre-allocate height, no jitter). */
  streaming?: boolean;
  /** Optional inline error detail (status === ERROR). */
  error?: string;
}

export interface Session {
  id: string;
  title: string;
  /** Epoch ms of last activity. */
  updatedAt: number;
}

/** Payload of the `chat_stream_chunk` Tauri event (P25 §九). */
export interface StreamChunk {
  session_id: string;
  chunk: string;
}

/** Payload of the `agent_status` Tauri event (P25 §九). */
export interface AgentStatusEvent {
  session_id: string;
  status: AgentStatus;
  /** Human label for THINKING rotation etc. */
  label?: string;
}
