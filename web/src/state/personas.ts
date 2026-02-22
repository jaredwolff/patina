import { signal } from "@preact/signals";
import type { Persona } from "../types";
import * as api from "../api";

export const personas = signal<Persona[]>([]);
export const modelTiers = signal<string[]>(["default"]);

export async function loadPersonas() {
  personas.value = await api.fetchPersonas();
}

export async function loadModelTiers() {
  modelTiers.value = await api.fetchModelTiers();
}

export function getPersona(key: string): Persona | null {
  return personas.value.find((p) => p.key === key) || null;
}

export function getPersonaName(key: string): string | null {
  const p = getPersona(key);
  return p ? p.name : key;
}

export const PRESET_COLORS = [
  "#e74c3c",
  "#e67e22",
  "#f1c40f",
  "#2ecc71",
  "#1abc9c",
  "#3498db",
  "#9b59b6",
  "#e91e63",
  "#795548",
  "#607d8b",
];
