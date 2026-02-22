import { route, navigate, type Route } from "../router";
import { sessions, activeChatId, unreadChats } from "../state/sessions";
import { personas } from "../state/personas";
import type { Persona, Session } from "../types";
import css from "./Sidebar.module.css";
import { useState, useRef, useEffect } from "preact/hooks";

const pages: { key: Route; label: string }[] = [
  { key: "chats", label: "Chats" },
  { key: "tasks", label: "Tasks" },
  { key: "usage", label: "Usage" },
];

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
  onToggleSidebar: () => void;
}

export function Sidebar({
  onNewChat,
  onSwitchChat,
  onDeleteChat,
  onManagePersonas,
  sidebarHidden,
}: SidebarProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  const currentRoute = route.value;
  const currentPage = pages.find((p) => p.key === currentRoute) || pages[0];
  const sessionList = sessions.value;
  const activeId = activeChatId.value;
  const unread = unreadChats.value;
  const personaList = personas.value;

  const showSessions = currentRoute === "chats";

  // Close menu on outside click
  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    document.addEventListener("click", handler);
    return () => document.removeEventListener("click", handler);
  }, [menuOpen]);

  if (sidebarHidden) return null;

  return (
    <aside class={css.sidebar}>
      <div class={css.header}>
        <div class={css.titleWrap} ref={menuRef}>
          <button
            class={css.menuBtn}
            onClick={(e) => {
              e.stopPropagation();
              setMenuOpen(!menuOpen);
            }}
          >
            <h2>{currentPage.label}</h2>
            <span class={css.chevron}>&#9662;</span>
          </button>
          {menuOpen && (
            <div class="dropdown">
              {pages.map((p) => (
                <button
                  key={p.key}
                  class={`dropdown-item${p.key === currentRoute ? " active" : ""}`}
                  onClick={() => {
                    navigate(p.key);
                    setMenuOpen(false);
                  }}
                >
                  {p.label}
                </button>
              ))}
            </div>
          )}
        </div>
        <div class={css.actions}>
          {showSessions && (
            <>
              <button
                class={css.iconBtn}
                title="Manage Personas"
                onClick={onManagePersonas}
              >
                &#9881;
              </button>
              <button
                class={css.newChatBtn}
                title="New Chat"
                onClick={onNewChat}
              >
                +
              </button>
            </>
          )}
        </div>
      </div>
      {showSessions && (
        <div class={css.sessionList}>
          {sessionList.map((s) => {
            const persona = getPersonaForSession(s, personaList);
            const isActive = s.id === activeId;
            return (
              <div
                key={s.id}
                class={`${css.sessionItem}${isActive ? ` ${css.active}` : ""}`}
                onClick={() => {
                  onSwitchChat(s.id);
                }}
              >
                <div
                  class={css.avatar}
                  style={{
                    background: persona?.color || "#888",
                  }}
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
      )}
    </aside>
  );
}
