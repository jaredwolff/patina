import { sessions, activeChatId, unreadChats } from "../state/sessions";
import { personas } from "../state/personas";
import type { Persona, Session } from "../types";
import css from "./Sidebar.module.css";

function formatTime(iso: string): string {
  try {
    const d = new Date(iso);
    const now = new Date();
    if (d.toDateString() === now.toDateString()) {
      return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    }
    return d.toLocaleDateString([], { month: "short", day: "numeric" });
  } catch {
    return "";
  }
}

function getPersonaForSession(
  session: Session,
  personaList: Persona[],
): Persona | null {
  if (!session.persona || !personaList.length) return null;
  return personaList.find((p) => p.key === session.persona) || null;
}

interface SidebarProps {
  onNewChat: () => void;
  onSwitchChat: (id: string) => void;
  onDeleteChat: (id: string) => void;
  onManagePersonas: () => void;
  sidebarHidden: boolean;
}

export function Sidebar({
  onNewChat,
  onSwitchChat,
  onDeleteChat,
  onManagePersonas,
  sidebarHidden,
}: SidebarProps) {
  const sessionList = sessions.value;
  const activeId = activeChatId.value;
  const unread = unreadChats.value;
  const personaList = personas.value;

  if (sidebarHidden) return null;

  return (
    <aside class={css.sidebar}>
      <div class={css.header}>
        <h2 class={css.title}>Chats</h2>
        <div class={css.actions}>
          <button
            class={css.iconBtn}
            title="Manage Personas"
            onClick={onManagePersonas}
          >
            &#9881;
          </button>
          <button class={css.newChatBtn} title="New Chat" onClick={onNewChat}>
            +
          </button>
        </div>
      </div>
      <div class={css.sessionList}>
        {sessionList.map((s) => {
          const persona = getPersonaForSession(s, personaList);
          const isActive = s.id === activeId;
          return (
            <div
              key={s.id}
              class={`${css.sessionItem}${isActive ? ` ${css.active}` : ""}`}
              onClick={() => onSwitchChat(s.id)}
            >
              <div
                class={css.avatar}
                style={{ background: persona?.color || "#888" }}
              >
                {persona?.name ? persona.name.charAt(0) : "P"}
              </div>
              <div class={css.content}>
                <div class={css.title}>
                  {unread[s.id] && <span class={css.unreadDot} />}
                  <span>{s.title || "New Chat"}</span>
                </div>
                {s.updatedAt && (
                  <div class={css.time}>{formatTime(s.updatedAt)}</div>
                )}
              </div>
              <button
                class={css.deleteBtn}
                title="Delete chat"
                onClick={(e) => {
                  e.stopPropagation();
                  if (confirm("Delete this chat?")) {
                    onDeleteChat(s.id);
                  }
                }}
              >
                &times;
              </button>
            </div>
          );
        })}
      </div>
    </aside>
  );
}
