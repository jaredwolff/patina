import { useRef, useEffect } from "preact/hooks";
import {
  messages,
  streamingText,
  isStreaming,
  showThinking,
} from "../state/messages";
import { MessageBubble } from "./MessageBubble";
import { ThinkingIndicator } from "./ThinkingIndicator";
import { renderMarkdown } from "../lib/markdown";

interface MessageListProps {
  onScroll?: (e: Event) => void;
}

export function MessageList({ onScroll }: MessageListProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const msgList = messages.value;
  const streaming = streamingText.value;
  const isStreamingActive = isStreaming.value;
  const thinking = showThinking.value;

  useEffect(() => {
    const el = containerRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }, [msgList, streaming, thinking]);

  return (
    <div
      ref={containerRef}
      data-messages
      style={containerStyle}
      onScroll={onScroll}
    >
      {msgList.map((msg, i) => (
        <MessageBubble key={i} message={msg} />
      ))}
      {isStreamingActive && streaming && (
        <div
          class="message assistant"
          dangerouslySetInnerHTML={{ __html: renderMarkdown(streaming) }}
        />
      )}
      {thinking && <ThinkingIndicator />}
    </div>
  );
}

const containerStyle: Record<string, string> = {
  flex: "1",
  overflowY: "auto",
  padding: "16px",
  display: "flex",
  flexDirection: "column",
  gap: "12px",
};
