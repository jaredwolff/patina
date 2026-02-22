import { signal } from "@preact/signals";

export type Route = "chats" | "tasks" | "usage";

const validRoutes: Route[] = ["chats", "tasks", "usage"];

function parseHash(): Route {
  const hash = window.location.hash.replace(/^#\/?/, "");
  return validRoutes.includes(hash as Route) ? (hash as Route) : "chats";
}

export const route = signal<Route>(parseHash());

export function navigate(to: Route) {
  window.location.hash = `/${to}`;
}

window.addEventListener("hashchange", () => {
  route.value = parseHash();
});
