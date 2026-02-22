import { signal } from "@preact/signals";
import type { UsageRow, UsageFilters } from "../types";
import * as api from "../api";

export const summaryRows = signal<UsageRow[]>([]);
export const dailyRows = signal<UsageRow[]>([]);
export const filters = signal<UsageFilters>({ models: [], providers: [], agents: [] });

// Summary card values
export const todayLabel = signal("-");
export const weekLabel = signal("-");
export const monthLabel = signal("-");
export const allTimeLabel = signal("-");

export function formatTokens(n: number | null | undefined): string {
  if (n === undefined || n === null) return "0";
  if (n >= 1000000) return (n / 1000000).toFixed(1) + "M";
  if (n >= 1000) return (n / 1000).toFixed(1) + "K";
  return String(n);
}

export function formatCost(n: number | null | undefined): string {
  if (n === null || n === undefined) return "\u2014";
  if (n < 0.01) return "<$0.01";
  return "$" + n.toFixed(2);
}

function formatCardValue(rows: UsageRow[]): string {
  let total = 0;
  let totalCost = 0;
  let hasCost = false;
  for (const r of rows) {
    total += r.total_tokens || 0;
    if (r.estimated_cost != null) {
      totalCost += r.estimated_cost;
      hasCost = true;
    }
  }
  let text = formatTokens(total);
  if (hasCost && totalCost > 0) {
    text += " (" + formatCost(totalCost) + ")";
  }
  return text;
}

export async function loadFilters() {
  try {
    filters.value = await api.fetchUsageFilters();
  } catch {
    // ignore
  }
}

export async function refreshUsage(params: Record<string, string>) {
  // Grouped summary
  try {
    const rows = await api.fetchUsageSummary(params);
    if (Array.isArray(rows)) summaryRows.value = rows;
  } catch {
    // ignore
  }

  // Daily breakdown (without groupBy)
  const dailyParams = { ...params };
  delete dailyParams.groupBy;
  try {
    const rows = await api.fetchUsageDaily(dailyParams);
    if (Array.isArray(rows)) dailyRows.value = rows;
  } catch {
    // ignore
  }

  // Summary cards
  await fetchSummaryCards();
}

async function fetchSummaryCards() {
  const now = new Date();
  const todayStr = now.toISOString().slice(0, 10);
  const dayStart = todayStr + "T00:00:00Z";
  const dayEnd = todayStr + "T23:59:59Z";

  // Today
  try {
    const rows = await api.fetchUsageDaily({ from: dayStart, to: dayEnd });
    todayLabel.value = formatCardValue(rows);
  } catch {
    todayLabel.value = "-";
  }

  // This week (Monday start)
  const weekDay = now.getDay();
  const mondayOffset = weekDay === 0 ? 6 : weekDay - 1;
  const monday = new Date(now);
  monday.setDate(now.getDate() - mondayOffset);
  const weekStart = monday.toISOString().slice(0, 10) + "T00:00:00Z";
  try {
    const rows = await api.fetchUsageDaily({ from: weekStart, to: dayEnd });
    weekLabel.value = formatCardValue(rows);
  } catch {
    weekLabel.value = "-";
  }

  // This month
  const monthStart = todayStr.slice(0, 7) + "-01T00:00:00Z";
  try {
    const rows = await api.fetchUsageDaily({ from: monthStart, to: dayEnd });
    monthLabel.value = formatCardValue(rows);
  } catch {
    monthLabel.value = "-";
  }

  // All time
  try {
    const rows = await api.fetchUsageDaily({});
    allTimeLabel.value = formatCardValue(rows);
  } catch {
    allTimeLabel.value = "-";
  }
}
