import { signal } from "@preact/signals";
import type { Message } from "../types";

export const messages = signal<Message[]>([]);
export const streamingText = signal("");
export const isStreaming = signal(false);
export const isGenerating = signal(false);
export const showThinking = signal(false);

export function addMessage(role: Message["role"], content: string) {
  messages.value = [...messages.value, { role, content }];
}

export function clearMessages() {
  messages.value = [];
  streamingText.value = "";
  isStreaming.value = false;
  showThinking.value = false;
}

export function setHistory(msgs: Message[]) {
  messages.value = msgs;
}

export function startStreaming() {
  isStreaming.value = true;
  streamingText.value = "";
  showThinking.value = false;
}

export function appendStreamChunk(chunk: string) {
  if (!isStreaming.value) {
    startStreaming();
  }
  showThinking.value = false;
  streamingText.value += chunk;
}

export function finalizeStream(content: string) {
  isStreaming.value = false;
  streamingText.value = "";
  isGenerating.value = false;
  // Replace any streaming content with the final message
  messages.value = [...messages.value, { role: "assistant", content }];
}

export function finalizeStreamWithExisting() {
  if (isStreaming.value && streamingText.value) {
    messages.value = [
      ...messages.value,
      { role: "assistant", content: streamingText.value },
    ];
  }
  isStreaming.value = false;
  streamingText.value = "";
  isGenerating.value = false;
}
