import { useRef } from "preact/hooks";
import { isGenerating } from "../state/messages";
import { connectionStatus } from "../state/websocket";
import css from "./ChatInput.module.css";

interface ChatInputProps {
  onSend: (text: string) => void;
  onCancel: () => void;
}

export function ChatInput({ onSend, onCancel }: ChatInputProps) {
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const generating = isGenerating.value;
  const connected = connectionStatus.value === "connected";

  function handleSubmit(e: Event) {
    e.preventDefault();
    if (generating) {
      onCancel();
      return;
    }
    const el = inputRef.current;
    if (!el) return;
    const text = el.value.trim();
    if (!text) return;
    onSend(text);
    el.value = "";
    el.style.height = "auto";
  }

  function handleInput() {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 120) + "px";
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSubmit(e);
    }
  }

  return (
    <form class={css.form} onSubmit={handleSubmit}>
      <textarea
        ref={inputRef}
        class={css.input}
        placeholder="Type a message..."
        rows={1}
        autoComplete="off"
        disabled={generating}
        onInput={handleInput}
        onKeyDown={handleKeyDown}
      />
      <button
        type="submit"
        class={css.sendBtn}
        disabled={!generating && !connected}
      >
        {generating ? "Stop" : "Send"}
      </button>
    </form>
  );
}
