import { signal } from "@preact/signals";

export type RouteName = "chats" | "tasks" | "usage";

export interface ParsedRoute {
  name: RouteName;
  param: string | null;
}

const validRoutes: RouteName[] = ["chats", "tasks", "usage"];

function parseHash(): ParsedRoute {
  const raw = window.location.hash.replace(/^#\/?/, "");
  const [first, ...rest] = raw.split("/");
  const name = validRoutes.includes(first as RouteName)
    ? (first as RouteName)
    : "chats";
  const param = rest.join("/") || null;
  return { name, param };
}

export const route = signal<ParsedRoute>(parseHash());

export function navigate(name: RouteName, param?: string | null) {
  const hash = param ? `/${name}/${param}` : `/${name}`;
  window.location.hash = hash;
}

window.addEventListener("hashchange", () => {
  route.value = parseHash();
});
