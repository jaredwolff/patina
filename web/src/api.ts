import type { Persona, Task, UsageRow, UsageFilters } from "./types";

function buildQuery(params: Record<string, string>): string {
  const parts = Object.entries(params)
    .filter(([, v]) => v)
    .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`);
  return parts.length ? `?${parts.join("&")}` : "";
}

// Sessions

export interface ServerSession {
  key: string;
  title?: string;
  updated_at?: string;
  persona?: string;
}

export async function fetchSessions(): Promise<ServerSession[]> {
  const res = await fetch("/api/sessions");
  return res.json();
}

export async function deleteSession(id: string): Promise<void> {
  await fetch(`/api/sessions/${encodeURIComponent(id)}`, { method: "DELETE" });
}

// Personas

export async function fetchPersonas(): Promise<Persona[]> {
  try {
    const res = await fetch("/api/personas");
    return await res.json();
  } catch {
    return [];
  }
}

export async function createPersona(data: Partial<Persona>): Promise<Persona> {
  const res = await fetch("/api/personas", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
  if (!res.ok) {
    const e = await res.json();
    throw new Error(e.error);
  }
  return res.json();
}

export async function updatePersona(
  key: string,
  data: Partial<Persona>,
): Promise<Persona> {
  const res = await fetch(`/api/personas/${encodeURIComponent(key)}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
  if (!res.ok) {
    const e = await res.json();
    throw new Error(e.error);
  }
  return res.json();
}

export async function deletePersona(key: string): Promise<void> {
  const res = await fetch(`/api/personas/${encodeURIComponent(key)}`, {
    method: "DELETE",
  });
  if (!res.ok) {
    const e = await res.json();
    throw new Error(e.error);
  }
}

export async function generatePersonaPrompt(
  data: Record<string, string>,
): Promise<{ preamble: string }> {
  const res = await fetch("/api/personas/generate-prompt", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
  return res.json();
}

export async function fetchModelTiers(): Promise<string[]> {
  try {
    const res = await fetch("/api/model-tiers");
    return await res.json();
  } catch {
    return ["default"];
  }
}

// Usage

export async function fetchUsageSummary(
  params: Record<string, string>,
): Promise<UsageRow[]> {
  const res = await fetch(`/api/usage/summary${buildQuery(params)}`);
  return res.json();
}

export async function fetchUsageDaily(
  params: Record<string, string>,
): Promise<UsageRow[]> {
  const res = await fetch(`/api/usage/daily${buildQuery(params)}`);
  return res.json();
}

export async function fetchUsageFilters(): Promise<UsageFilters> {
  const res = await fetch("/api/usage/filters");
  return res.json();
}

// Tasks

export async function fetchTasks(): Promise<Task[]> {
  const res = await fetch("/api/tasks");
  return res.json();
}

export async function createTask(
  data: Partial<Task>,
): Promise<Task> {
  const res = await fetch("/api/tasks", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
  return res.json();
}

export async function updateTask(
  id: string,
  data: Partial<Task>,
): Promise<void> {
  await fetch(`/api/tasks/${id}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
}

export async function deleteTask(id: string): Promise<void> {
  await fetch(`/api/tasks/${id}`, { method: "DELETE" });
}

export async function moveTask(
  id: string,
  status: string,
): Promise<void> {
  await fetch(`/api/tasks/${id}/move`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ status }),
  });
}

export async function assignTask(
  id: string,
  assignee: string | null,
): Promise<void> {
  await fetch(`/api/tasks/${id}/assign`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ assignee }),
  });
}
