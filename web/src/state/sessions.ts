import { signal } from "@preact/signals";
import type { Session } from "../types";
import * as api from "../api";

export const sessions = signal<Session[]>([]);
export const activeChatId = signal<string | null>(null);
export const unreadChats = signal<Record<string, boolean>>({});

const STORAGE_KEY = "patina-sessions";

export function loadSessions() {
  const old = localStorage.getItem("patina-session");
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored) {
    try {
      sessions.value = JSON.parse(stored);
    } catch {
      sessions.value = [];
    }
  } else if (old) {
    sessions.value = [
      { id: old, title: "Chat", updatedAt: new Date().toISOString() },
    ];
    localStorage.removeItem("patina-session");
  }
  saveSessions();
}

export function saveSessions() {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(sessions.value));
}

export function findSession(id: string): Session | null {
  return sessions.value.find((s) => s.id === id) || null;
}

export function addSession(session: Session) {
  sessions.value = [session, ...sessions.value];
  saveSessions();
}

export function removeSession(id: string) {
  sessions.value = sessions.value.filter((s) => s.id !== id);
  const u = { ...unreadChats.value };
  delete u[id];
  unreadChats.value = u;
  saveSessions();
}

export function updateSessionTitle(id: string, firstMessage: string) {
  const s = findSession(id);
  if (s && s.title === "New Chat") {
    s.title =
      firstMessage.length > 50
        ? firstMessage.substring(0, 50) + "..."
        : firstMessage;
    sessions.value = [...sessions.value];
    saveSessions();
  }
}

export function updateSessionTime(id: string) {
  const s = findSession(id);
  if (s) {
    s.updatedAt = new Date().toISOString();
    sessions.value = [...sessions.value];
    saveSessions();
  }
}

export function markUnread(id: string) {
  unreadChats.value = { ...unreadChats.value, [id]: true };
}

export function clearUnread(id: string) {
  const u = { ...unreadChats.value };
  delete u[id];
  unreadChats.value = u;
}

export function getSessionPersona(id: string): string | null {
  const s = findSession(id);
  return s?.persona || null;
}

export function generateUUID(): string {
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    return (c === "x" ? r : (r & 0x3) | 0x8).toString(16);
  });
}

export async function syncSessions() {
  try {
    const serverSessions = await api.fetchSessions();
    const serverIds: Record<string, boolean> = {};
    const current = sessions.value;

    for (const ss of serverSessions) {
      serverIds[ss.key] = true;
      const existing = current.find((s) => s.id === ss.key);
      if (!existing) {
        current.push({
          id: ss.key,
          title: ss.title || ss.key.slice(0, 8),
          updatedAt: ss.updated_at || new Date().toISOString(),
          persona: ss.persona || null,
        });
      } else {
        if (ss.title && ss.title !== existing.title) {
          existing.title = ss.title;
        }
        if (ss.persona) {
          existing.persona = ss.persona;
        }
      }
    }

    sessions.value = [...current];
    saveSessions();
  } catch {
    // ignore sync failures
  }
}
