import type { AgentStatus } from "@/types/chat";

/** Short UI labels for agent states (P25 §五). No spinners — text only. */
export const AGENT_STATUS_LABEL: Record<AgentStatus, string> = {
  IDLE: "就绪",
  THINKING: "规划任务",
  AWAITING_CONFIRMATION: "等待确认",
  EXECUTING_TOOL: "执行工具",
  STREAMING_ANSWER: "回答中",
  UNCERTAIN: "不确定",
  ERROR: "出错",
};

/** THINKING rotation phrases — 3s/round, no spinning icon (P25 §五). */
export const THINKING_PHRASES: string[] = [
  "规划任务",
  "查阅知识库",
  "检索记忆",
  "选择技能",
];

/** Local-first trust signal shown in the sidebar (P25 §六). */
export const DATA_PATH_LABEL = "数据在本地 · ~/.caspian/";

/** Default data directory shorthand returned by the mock `get_data_path`. */
export const MOCK_DATA_PATH = "~/.caspian/";
