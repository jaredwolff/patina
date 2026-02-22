import { useState, useEffect } from "preact/hooks";
import {
  summaryRows,
  dailyRows,
  filters,
  todayLabel,
  weekLabel,
  monthLabel,
  allTimeLabel,
  loadFilters,
  refreshUsage,
} from "../state/usage";
import { UsageTable } from "./UsageTable";
import css from "./UsageView.module.css";

export function UsageView() {
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [model, setModel] = useState("");
  const [provider, setProvider] = useState("");
  const [agent, setAgent] = useState("");
  const [groupBy, setGroupBy] = useState("model");

  const filterData = filters.value;

  useEffect(() => {
    loadFilters();
    handleRefresh();
  }, []);

  function getParams(): Record<string, string> {
    const params: Record<string, string> = {};
    if (from) params.from = from + "T00:00:00Z";
    if (to) params.to = to + "T23:59:59Z";
    if (model) params.model = model;
    if (provider) params.provider = provider;
    if (agent) params.agent = agent;
    if (groupBy) params.groupBy = groupBy;
    return params;
  }

  function handleRefresh() {
    refreshUsage(getParams());
  }

  return (
    <div class={css.view}>
      <div class={css.content}>
        <div class={css.filters}>
          <label>
            From
            <input
              type="date"
              value={from}
              onInput={(e) => setFrom((e.target as HTMLInputElement).value)}
            />
          </label>
          <label>
            To
            <input
              type="date"
              value={to}
              onInput={(e) => setTo((e.target as HTMLInputElement).value)}
            />
          </label>
          <label>
            Model
            <select
              value={model}
              onChange={(e) => setModel((e.target as HTMLSelectElement).value)}
            >
              <option value="">All</option>
              {filterData.models.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          </label>
          <label>
            Provider
            <select
              value={provider}
              onChange={(e) =>
                setProvider((e.target as HTMLSelectElement).value)
              }
            >
              <option value="">All</option>
              {filterData.providers.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
          </label>
          <label>
            Agent
            <select
              value={agent}
              onChange={(e) => setAgent((e.target as HTMLSelectElement).value)}
            >
              <option value="">All</option>
              {filterData.agents.map((a) => (
                <option key={a} value={a}>
                  {a}
                </option>
              ))}
            </select>
          </label>
          <label>
            Group by
            <select
              value={groupBy}
              onChange={(e) =>
                setGroupBy((e.target as HTMLSelectElement).value)
              }
            >
              <option value="model">Model</option>
              <option value="provider">Provider</option>
              <option value="agent">Agent</option>
              <option value="session">Session</option>
              <option value="call_type">Call Type</option>
            </select>
          </label>
          <button
            class="btn-primary"
            style={{
              alignSelf: "flex-end",
              padding: "6px 16px",
              fontSize: "13px",
            }}
            onClick={handleRefresh}
          >
            Refresh
          </button>
        </div>
        <div class={css.summaryCards}>
          <div class={css.card}>
            <div class={css.cardLabel}>Today</div>
            <div class={css.cardValue}>{todayLabel.value}</div>
          </div>
          <div class={css.card}>
            <div class={css.cardLabel}>This Week</div>
            <div class={css.cardValue}>{weekLabel.value}</div>
          </div>
          <div class={css.card}>
            <div class={css.cardLabel}>This Month</div>
            <div class={css.cardValue}>{monthLabel.value}</div>
          </div>
          <div class={css.card}>
            <div class={css.cardLabel}>All Time</div>
            <div class={css.cardValue}>{allTimeLabel.value}</div>
          </div>
        </div>
        <h3>Summary</h3>
        <UsageTable rows={summaryRows.value} firstCol="group_key" />
        <h3>Daily Breakdown</h3>
        <UsageTable rows={dailyRows.value} firstCol="date" />
      </div>
    </div>
  );
}
