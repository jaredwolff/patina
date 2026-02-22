import type { ComponentChildren } from "preact";
import { route, navigate, type Route } from "../router";
import { connectionStatus, statusText } from "../state/websocket";
import css from "./Header.module.css";

const tabs: { key: Route; label: string }[] = [
  { key: "chats", label: "Chats" },
  { key: "tasks", label: "Tasks" },
  { key: "usage", label: "Usage" },
];

interface HeaderProps {
  onToggleSidebar?: () => void;
  showSidebarToggle?: boolean;
  children?: ComponentChildren;
}

export function Header({
  onToggleSidebar,
  showSidebarToggle,
  children,
}: HeaderProps) {
  const currentRoute = route.value;
  const status = connectionStatus.value;
  const statusLabel = statusText.value;

  return (
    <header class={css.header}>
      {showSidebarToggle && (
        <button
          class={css.hamburger}
          title="Toggle sidebar"
          onClick={onToggleSidebar}
        >
          &#9776;
        </button>
      )}
      <span class={css.brand}>Patina</span>
      <nav class={css.nav}>
        {tabs.map((t) => (
          <button
            key={t.key}
            class={t.key === currentRoute ? css.navTabActive : css.navTab}
            onClick={() => navigate(t.key)}
          >
            {t.label}
          </button>
        ))}
      </nav>
      {children && <div class={css.actions}>{children}</div>}
      <span class={`status ${status}`}>{statusLabel}</span>
    </header>
  );
}
