import { renderMarkdown } from "../lib/markdown";
import type { Message } from "../types";

interface MessageBubbleProps {
  message: Message;
}

export function MessageBubble({ message }: MessageBubbleProps) {
  if (message.role === "user" || message.role === "system") {
    return <div class={`message ${message.role}`}>{message.content}</div>;
  }

  return (
    <div
      class="message assistant"
      dangerouslySetInnerHTML={{ __html: renderMarkdown(message.content) }}
    />
  );
}
