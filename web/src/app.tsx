import { useState } from "preact/hooks";
import { route, navigate } from "./router";
import { Header } from "./components/Header";
import { ChatView } from "./components/ChatView";
import { UsageView } from "./components/UsageView";
import { TasksView } from "./components/TasksView";
import { PersonaPicker } from "./components/PersonaPicker";
import { PersonaManager } from "./components/PersonaManager";
import { PersonaEditor } from "./components/PersonaEditor";
import {
  activeChatId,
  sessions,
  getSessionPersona,
  clearUnread,
  removeSession,
  generateUUID,
  addSession,
} from "./state/sessions";
import { personas, getPersona } from "./state/personas";
import { clearMessages } from "./state/messages";
import { send } from "./state/websocket";
import { taskEditorOpen } from "./state/tasks";
import type { Persona } from "./types";

function closeSidebarMobile(setSidebarHidden: (v: boolean) => void) {
  if (window.innerWidth <= 768) {
    setSidebarHidden(true);
  }
}

export function App() {
  const [sidebarHidden, setSidebarHidden] = useState(window.innerWidth <= 768);
  const [managerOpen, setManagerOpen] = useState(false);
  const [editorOpen, setEditorOpen] = useState(false);
  const [editingPersona, setEditingPersona] = useState<Persona | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);

  const currentRoute = route.value.name;
  const routeParam = route.value.param;

  function finishCreateChat(personaKey: string | null) {
    const id = generateUUID();
    addSession({
      id,
      title: "New Chat",
      updatedAt: new Date().toISOString(),
      persona: personaKey,
    });
    send({ type: "create_session", chatId: id, content: personaKey || "" });
    activeChatId.value = id;
    clearMessages();
    navigate("chats", id);
    closeSidebarMobile(setSidebarHidden);
  }

  function handleNewChat() {
    const personaList = personas.value;
    if (personaList.length > 0) {
      setPickerOpen(true);
    } else {
      finishCreateChat(null);
    }
  }

  function handleSwitchChat(id: string) {
    activeChatId.value = id;
    clearUnread(id);
    clearMessages();
    send({ type: "get_history", chatId: id });
    navigate("chats", id);
    closeSidebarMobile(setSidebarHidden);
  }

  function handleDeleteChat(id: string) {
    fetch(`/api/sessions/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }).catch(() => {});
    send({ type: "delete_session", chatId: id });
    removeSession(id);

    if (id === activeChatId.value) {
      if (sessions.value.length > 0) {
        handleSwitchChat(sessions.value[0].id);
      } else {
        activeChatId.value = null;
        clearMessages();
        handleNewChat();
      }
    }
  }

  function handleEditPersona(persona: Persona | null) {
    setEditingPersona(persona);
    setEditorOpen(true);
  }

  function handleCopyChatId() {
    const chatId = activeChatId.value;
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

  function renderHeaderActions() {
    switch (currentRoute) {
      case "chats": {
        const chatId = activeChatId.value;
        const personaKey = chatId ? getSessionPersona(chatId) : null;
        const persona = personaKey ? getPersona(personaKey) : null;
        return (
          <>
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
            {chatId && (
              <span
                style={{
                  fontSize: "11px",
                  fontFamily: "monospace",
                  color: "var(--text-secondary)",
                  opacity: 0.5,
                  cursor: "pointer",
                  userSelect: "none",
                }}
                title="Click to copy session key"
                onClick={handleCopyChatId}
              >
                {chatId.slice(0, 8)}
              </span>
            )}
          </>
        );
      }
      case "tasks":
        return (
          <button
            class="btn-primary btn-sm"
            onClick={() => {
              taskEditorOpen.value = true;
            }}
          >
            + New Task
          </button>
        );
      default:
        return null;
    }
  }

  const showSidebarToggle = currentRoute === "chats";

  function renderView() {
    switch (currentRoute) {
      case "tasks":
        return <TasksView initialTaskId={routeParam} />;
      case "usage":
        return <UsageView />;
      case "chats":
      default:
        return (
          <ChatView
            sidebarHidden={sidebarHidden}
            onNewChat={handleNewChat}
            onSwitchChat={handleSwitchChat}
            onDeleteChat={handleDeleteChat}
            onManagePersonas={() => setManagerOpen(true)}
          />
        );
    }
  }

  return (
    <>
      <Header
        onToggleSidebar={() => setSidebarHidden(!sidebarHidden)}
        showSidebarToggle={showSidebarToggle}
      >
        {renderHeaderActions()}
      </Header>
      <div class="mainArea">{renderView()}</div>
      <PersonaPicker
        visible={pickerOpen}
        personas={personas.value}
        onSelect={(key) => {
          setPickerOpen(false);
          finishCreateChat(key);
        }}
        onCancel={() => {
          setPickerOpen(false);
          if (sessions.value.length === 0) {
            finishCreateChat(null);
          }
        }}
      />
      <PersonaManager
        visible={managerOpen}
        onClose={() => setManagerOpen(false)}
        onEdit={handleEditPersona}
      />
      <PersonaEditor
        visible={editorOpen}
        persona={editingPersona}
        onClose={() => setEditorOpen(false)}
      />
    </>
  );
}
