import { useState } from "preact/hooks";
import type { UsageRow } from "../types";
import { formatTokens, formatCost } from "../state/usage";
import css from "./UsageView.module.css";

interface UsageTableProps {
  rows: UsageRow[];
  firstCol: "group_key" | "date";
}

const columns: { key: string; label: string }[] = [
  { key: "first", label: "Group" },
  { key: "calls", label: "Calls" },
  { key: "input_tokens", label: "Input" },
  { key: "output_tokens", label: "Output" },
  { key: "total_tokens", label: "Total" },
  { key: "cached_input_tokens", label: "Cached" },
  { key: "estimated_cost", label: "Cost" },
];

function parseTokenValue(str: string): number {
  const s = str.trim();
  if (s.endsWith("M")) return parseFloat(s) * 1000000;
  if (s.endsWith("K")) return parseFloat(s) * 1000;
  return parseFloat(s) || 0;
}

function getCellValue(row: UsageRow, key: string, firstCol: string): string {
  if (key === "first") {
    return String((row as unknown as Record<string, unknown>)[firstCol] || "-");
  }
  if (key === "estimated_cost") {
    return formatCost(row.estimated_cost);
  }
  const val = (row as unknown as Record<string, unknown>)[key];
  return formatTokens(val as number);
}

export function UsageTable({ rows, firstCol }: UsageTableProps) {
  const [sortKey, setSortKey] = useState("total_tokens");
  const [sortAsc, setSortAsc] = useState(false);

  const cols = columns.map((c) =>
    c.key === "first"
      ? { ...c, label: firstCol === "date" ? "Date" : "Group" }
      : c,
  );

  function handleSort(key: string) {
    if (key === sortKey) {
      setSortAsc(!sortAsc);
    } else {
      setSortKey(key);
      setSortAsc(false);
    }
  }

  const sortedRows = [...rows].sort((a, b) => {
    const aVal = getCellValue(a, sortKey, firstCol);
    const bVal = getCellValue(b, sortKey, firstCol);
    const aNum = parseTokenValue(aVal);
    const bNum = parseTokenValue(bVal);
    if (!isNaN(aNum) && !isNaN(bNum)) {
      return sortAsc ? aNum - bNum : bNum - aNum;
    }
    return sortAsc ? aVal.localeCompare(bVal) : bVal.localeCompare(aVal);
  });

  if (!rows.length) {
    return <div class={css.noData}>No data</div>;
  }

  return (
    <div class={css.tableWrap}>
      <table class={css.table}>
        <thead>
          <tr>
            {cols.map((c) => (
              <th
                key={c.key}
                class={
                  sortKey === c.key
                    ? sortAsc
                      ? css.sortAsc
                      : css.sortDesc
                    : ""
                }
                onClick={() => handleSort(c.key)}
              >
                {c.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sortedRows.map((row, i) => (
            <tr key={i}>
              {cols.map((c) => (
                <td key={c.key}>{getCellValue(row, c.key, firstCol)}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
