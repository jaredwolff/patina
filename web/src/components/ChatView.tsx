import { useEffect } from "preact/hooks";
import {
  activeChatId,
  getSessionPersona,
  updateSessionTitle,
  updateSessionTime,
} from "../state/sessions";
import { connectionStatus, statusText, send } from "../state/websocket";
import {
  addMessage,
  isGenerating,
  showThinking as showThinkingSignal,
} from "../state/messages";
import { getPersona } from "../state/personas";
import { MessageList } from "./MessageList";
import { ChatInput } from "./ChatInput";
import css from "./ChatView.module.css";

export interface ChatViewProps {
  sidebarHidden: boolean;
  onToggleSidebar: () => void;
}

export function ChatView({ sidebarHidden, onToggleSidebar }: ChatViewProps) {
  const chatId = activeChatId.value;
  const status = connectionStatus.value;
  const statusLabel = statusText.value;
  const personaKey = chatId ? getSessionPersona(chatId) : null;
  const persona = personaKey ? getPersona(personaKey) : null;

  function handleCancel() {
    if (chatId) {
      send({ type: "cancel", chatId });
    }
    isGenerating.value = false;
    showThinkingSignal.value = false;
  }

  // Global ESC to cancel generation
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" && isGenerating.value) {
        e.preventDefault();
        handleCancel();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [chatId]);

  function handleSend(text: string) {
    if (!chatId) return;

    addMessage("user", text);
    showThinkingSignal.value = true;
    updateSessionTitle(chatId, text);
    updateSessionTime(chatId);

    const msg: Record<string, unknown> = {
      type: "message",
      content: text,
      chatId,
    };
    if (personaKey) msg.persona = personaKey;

    send(msg as never);
    isGenerating.value = true;
  }

  function handleCopyChatId() {
    if (!chatId) return;
    const sessionKey = "web:" + chatId;
    navigator.clipboard.writeText(sessionKey).catch(() => {
      const ta = document.createElement("textarea");
      ta.value = sessionKey;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      document.body.removeChild(ta);
    });
  }

  function handleMessagesScroll(e: Event) {
    const el = e.target as HTMLElement;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 100;
    const btn = document.getElementById("scroll-bottom-btn");
    if (btn) btn.classList.toggle("hidden", nearBottom);
  }

  function scrollToBottom() {
    // Find the messages container (MessageList's root div)
    const el = document.querySelector<HTMLElement>("[data-messages]");
    if (el) el.scrollTop = el.scrollHeight;
  }

  return (
    <div class={css.chatArea}>
      <header class={css.header}>
        {sidebarHidden && (
          <button
            class={css.sidebarToggle}
            title="Toggle sidebar"
            onClick={onToggleSidebar}
            style={{ display: "block" }}
          >
            &#9776;
          </button>
        )}
        <h1>
          Patina
          {persona && (
            <span class="persona-badge">
              <span
                class="header-avatar"
                style={{ background: persona.color || "#888" }}
              >
                {persona.name?.charAt(0) || "P"}
              </span>
              <span>{persona.name || persona.key}</span>
            </span>
          )}
        </h1>
        <span class={`status ${status}`}>{statusLabel}</span>
        {chatId && (
          <span
            class={css.chatId}
            title="Click to copy session key"
            onClick={handleCopyChatId}
          >
            {chatId.slice(0, 8)}
          </span>
        )}
      </header>
      <MessageList onScroll={handleMessagesScroll} />
      <button
        id="scroll-bottom-btn"
        class={`${css.scrollBtn} hidden`}
        title="Scroll to bottom"
        onClick={scrollToBottom}
      >
        &#x2193;
      </button>
      <ChatInput onSend={handleSend} onCancel={handleCancel} />
    </div>
  );
}
