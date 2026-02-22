import { signal } from "@preact/signals";
import type { Task } from "../types";
import * as api from "../api";

export const tasks = signal<Task[]>([]);
export const taskEditorOpen = signal(false);

export async function loadTasks() {
  try {
    tasks.value = await api.fetchTasks();
  } catch (e) {
    console.error("Failed to load tasks:", e);
  }
}
