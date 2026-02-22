export interface Session {
  id: string;
  title: string;
  updatedAt: string;
  persona?: string | null;
}

export interface Message {
  role: "user" | "assistant" | "system";
  content: string;
}

export interface Persona {
  key: string;
  name: string;
  description?: string;
  color?: string;
  preamble?: string;
  modelTier?: string;
}

export interface Task {
  id: string;
  title: string;
  description?: string;
  status: "backlog" | "todo" | "in_progress" | "done";
  priority: "low" | "medium" | "high" | "urgent";
  assignee?: string | null;
  tags?: string[];
}

export interface UsageRow {
  group_key?: string;
  date?: string;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  cached_input_tokens: number;
  estimated_cost: number | null;
}

export interface UsageFilters {
  models: string[];
  providers: string[];
  agents: string[];
}

// WebSocket message types (server → client)
export type WsMessage =
  | { type: "connected" }
  | { type: "history"; chatId: string; messages: Message[] }
  | { type: "text_delta"; chatId: string; content: string }
  | { type: "message"; chatId: string; content: string }
  | { type: "user_message"; chatId: string; content: string }
  | { type: "thinking"; chatId: string }
  | {
      type: "session_created";
      chatId: string;
      content?: string;
      timestamp?: string;
    }
  | { type: "session_deleted"; chatId: string }
  | { type: "task_history"; chatId: string; messages: Message[] }
  | { type: "error"; content: string };

// WebSocket message types (client → server)
export type WsOutMessage =
  | {
      type: "message";
      chatId: string;
      content: string;
      persona?: string | null;
    }
  | { type: "get_history"; chatId: string }
  | { type: "get_task_history"; chatId: string }
  | { type: "cancel"; chatId: string }
  | { type: "create_session"; chatId: string; content: string }
  | { type: "delete_session"; chatId: string }
  | { type: "task_message"; chatId: string; content: string };
