import { useEffect } from "preact/hooks";
import {
  activeChatId,
  getSessionPersona,
  updateSessionTitle,
  updateSessionTime,
} from "../state/sessions";
import { send } from "../state/websocket";
import {
  addMessage,
  isGenerating,
  showThinking as showThinkingSignal,
} from "../state/messages";
import { Sidebar } from "./Sidebar";
import { MessageList } from "./MessageList";
import { ChatInput } from "./ChatInput";
import css from "./ChatView.module.css";

interface ChatViewProps {
  sidebarHidden: boolean;
  onNewChat: () => void;
  onSwitchChat: (id: string) => void;
  onDeleteChat: (id: string) => void;
  onManagePersonas: () => void;
}

export function ChatView({
  sidebarHidden,
  onNewChat,
  onSwitchChat,
  onDeleteChat,
  onManagePersonas,
}: ChatViewProps) {
  const chatId = activeChatId.value;
  const personaKey = chatId ? getSessionPersona(chatId) : null;

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

  function handleMessagesScroll(e: Event) {
    const el = e.target as HTMLElement;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 100;
    const btn = document.getElementById("scroll-bottom-btn");
    if (btn) btn.classList.toggle("hidden", nearBottom);
  }

  function scrollToBottom() {
    const el = document.querySelector<HTMLElement>("[data-messages]");
    if (el) el.scrollTop = el.scrollHeight;
  }

  return (
    <div class={css.chatArea}>
      <div class={css.chatBody}>
        <Sidebar
          onNewChat={onNewChat}
          onSwitchChat={onSwitchChat}
          onDeleteChat={onDeleteChat}
          onManagePersonas={onManagePersonas}
          sidebarHidden={sidebarHidden}
        />
        <div class={css.chatMain}>
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
      </div>
    </div>
  );
}
