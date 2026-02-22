import { signal } from "@preact/signals";
import type { WsMessage, WsOutMessage } from "../types";
import {
  activeChatId,
  findSession,
  addSession,
  removeSession,
  updateSessionTitle,
  updateSessionTime,
  sessions,
  markUnread,
} from "./sessions";
import {
  addMessage,
  clearMessages,
  setHistory,
  appendStreamChunk,
  finalizeStream,
  isStreaming,
  isGenerating,
  showThinking as showThinkingSignal,
} from "./messages";

export type ConnectionStatus = "connected" | "disconnected" | "reconnecting";
export const connectionStatus = signal<ConnectionStatus>("disconnected");
export const statusText = signal("disconnected");

let ws: WebSocket | null = null;
let reconnectDelay = 1000;

// Task detail streaming state (managed separately from chat)
export const activeTaskId = signal<string | null>(null);
export const taskMessages = signal<
  { role: "user" | "assistant" | "system"; content: string }[]
>([]);
export const taskStreamingText = signal("");
export const taskIsStreaming = signal(false);

export function send(msg: WsOutMessage) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(msg));
  }
}

export function connectWs() {
  if (ws) return;

  connectionStatus.value = "reconnecting";
  statusText.value = "connecting...";

  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  let url = `${proto}//${location.host}/ws`;

  const password = localStorage.getItem("patina-password");
  if (password) url += `?password=${encodeURIComponent(password)}`;

  ws = new WebSocket(url);

  ws.onopen = () => {
    connectionStatus.value = "connected";
    statusText.value = "connected";
    reconnectDelay = 1000;

    const chatId = activeChatId.value;
    if (chatId) {
      send({ type: "get_history", chatId });
    }
  };

  ws.onmessage = (evt) => {
    let data: WsMessage;
    try {
      data = JSON.parse(evt.data);
    } catch {
      return;
    }

    handleMessage(data);
  };

  ws.onclose = (evt) => {
    ws = null;
    showThinkingSignal.value = false;

    if (evt.code === 4001) {
      connectionStatus.value = "disconnected";
      statusText.value = "auth failed";
      promptPassword();
      return;
    }

    connectionStatus.value = "disconnected";
    statusText.value = "disconnected";
    setTimeout(connectWs, reconnectDelay);
    reconnectDelay = Math.min(reconnectDelay * 2, 30000);
  };

  ws.onerror = () => {};
}

function handleMessage(data: WsMessage) {
  const chatId = activeChatId.value;
  const taskId = activeTaskId.value;

  switch (data.type) {
    case "connected":
      break;

    case "history":
      if (data.chatId === chatId && data.messages?.length) {
        clearMessages();
        setHistory(data.messages);
      }
      break;

    case "text_delta":
      if (data.chatId === taskId && data.content) {
        taskIsStreaming.value = true;
        taskStreamingText.value += data.content;
      } else if (data.chatId === chatId && data.content) {
        appendStreamChunk(data.content);
      }
      break;

    case "message":
      if (data.chatId === taskId) {
        if (taskIsStreaming.value) {
          taskIsStreaming.value = false;
          taskMessages.value = [
            ...taskMessages.value,
            { role: "assistant", content: data.content },
          ];
          taskStreamingText.value = "";
        } else {
          taskMessages.value = [
            ...taskMessages.value,
            { role: "assistant", content: data.content },
          ];
        }
        break;
      }
      isGenerating.value = false;
      if (data.chatId === chatId) {
        if (isStreaming.value) {
          finalizeStream(data.content);
        } else {
          addMessage("assistant", data.content);
        }
      } else if (data.chatId) {
        markUnread(data.chatId);
      }
      if (data.chatId) {
        updateSessionTime(data.chatId);
      }
      break;

    case "user_message":
      if (data.chatId === taskId) {
        taskMessages.value = [
          ...taskMessages.value,
          { role: "user", content: data.content },
        ];
      } else if (data.chatId === chatId) {
        addMessage("user", data.content);
      }
      if (data.chatId) {
        updateSessionTitle(data.chatId, data.content);
        updateSessionTime(data.chatId);
        if (data.chatId !== chatId) {
          markUnread(data.chatId);
        }
      }
      break;

    case "thinking":
      if (data.chatId === taskId) {
        // Task thinking handled in component
      } else if (data.chatId === chatId) {
        showThinkingSignal.value = true;
      }
      break;

    case "session_created":
      if (data.chatId && !findSession(data.chatId)) {
        addSession({
          id: data.chatId,
          title: "New Chat",
          updatedAt: data.timestamp || new Date().toISOString(),
          persona: data.content || null,
        });
      }
      break;

    case "session_deleted":
      if (data.chatId) {
        removeSession(data.chatId);
        if (data.chatId === chatId) {
          if (sessions.value.length > 0) {
            activeChatId.value = sessions.value[0].id;
          } else {
            activeChatId.value = null;
            clearMessages();
          }
        }
      }
      break;

    case "task_history":
      if (data.chatId === taskId) {
        taskMessages.value = data.messages || [];
      }
      break;

    case "error":
      if (data.content?.includes("Authentication")) {
        promptPassword();
      }
      addMessage("system", data.content || "Unknown error");
      break;
  }
}

function promptPassword() {
  const pw = prompt("Enter password:");
  if (pw !== null) {
    localStorage.setItem("patina-password", pw);
    if (ws) {
      ws.close();
      ws = null;
    }
    connectWs();
  }
}

export function disconnectWs() {
  if (ws) {
    ws.close();
    ws = null;
  }
}
