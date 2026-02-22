import { useState, useEffect } from "preact/hooks";
import { tasks, taskEditorOpen, loadTasks } from "../state/tasks";
import { personas } from "../state/personas";
import { navigate } from "../router";
import { TaskEditor } from "./TaskEditor";
import { TaskDetail } from "./TaskDetail";
import * as api from "../api";
import type { Task } from "../types";
import css from "./TasksView.module.css";

const STATUSES: { key: Task["status"]; label: string }[] = [
  { key: "backlog", label: "Backlog" },
  { key: "todo", label: "Todo" },
  { key: "in_progress", label: "In Progress" },
  { key: "done", label: "Done" },
];

const PRIORITY_ORDER: Record<string, number> = {
  urgent: 0,
  high: 1,
  medium: 2,
  low: 3,
};

function priorityClass(p: string): string {
  switch (p) {
    case "low":
      return css.priorityLow;
    case "medium":
      return css.priorityMedium;
    case "high":
      return css.priorityHigh;
    case "urgent":
      return css.priorityUrgent;
    default:
      return css.priorityMedium;
  }
}

interface TasksViewProps {
  initialTaskId?: string | null;
}

export function TasksView({ initialTaskId }: TasksViewProps) {
  const [editingTask] = useState<Task | null>(null);
  const [detailTask, setDetailTask] = useState<Task | null>(null);
  const [dragOverStatus, setDragOverStatus] = useState<string | null>(null);

  const editorOpen = taskEditorOpen.value;
  const taskList = tasks.value;
  const personaList = personas.value;

  useEffect(() => {
    loadTasks();
  }, []);

  // Open task detail from URL param
  useEffect(() => {
    if (initialTaskId && taskList.length > 0) {
      const t = taskList.find((t) => t.id === initialTaskId);
      if (t) setDetailTask(t);
    }
  }, [initialTaskId, taskList]);

  const columns: Record<string, Task[]> = {
    backlog: [],
    todo: [],
    in_progress: [],
    done: [],
  };
  for (const t of taskList) {
    if (columns[t.status]) {
      columns[t.status].push(t);
    }
  }
  // Sort by priority within each column
  for (const col of Object.values(columns)) {
    col.sort(
      (a, b) =>
        (PRIORITY_ORDER[a.priority] ?? 3) - (PRIORITY_ORDER[b.priority] ?? 3),
    );
  }

  function handleDrop(e: DragEvent, newStatus: string) {
    e.preventDefault();
    setDragOverStatus(null);
    const taskId = e.dataTransfer?.getData("text/plain");
    if (!taskId || !newStatus) return;
    api.moveTask(taskId, newStatus).then(() => loadTasks());
  }

  function handleOpenTask(t: Task) {
    setDetailTask(t);
    navigate("tasks", t.id);
  }

  function handleCloseTask() {
    setDetailTask(null);
    navigate("tasks");
  }

  function getAssigneeColor(assigneeKey: string): string {
    const p = personaList.find((p) => p.key === assigneeKey);
    return p?.color || "var(--accent)";
  }

  return (
    <div class={css.view}>
      <div class={css.board}>
        {STATUSES.map(({ key, label }) => (
          <div key={key} class={css.column}>
            <div class={css.columnHeader}>
              <span class={css.columnTitle}>{label}</span>
              <span class={css.columnCount}>{columns[key].length}</span>
            </div>
            <div
              class={`${css.cards}${dragOverStatus === key ? ` ${css.dragOver}` : ""}`}
              onDragOver={(e) => {
                e.preventDefault();
                setDragOverStatus(key);
              }}
              onDragLeave={() => setDragOverStatus(null)}
              onDrop={(e) => handleDrop(e, key)}
            >
              {columns[key].map((t) => (
                <div
                  key={t.id}
                  class={css.card}
                  draggable
                  onDragStart={(e) => {
                    e.dataTransfer?.setData("text/plain", t.id);
                    (e.target as HTMLElement).classList.add(css.dragging);
                  }}
                  onDragEnd={(e) => {
                    (e.target as HTMLElement).classList.remove(css.dragging);
                  }}
                  onClick={() => handleOpenTask(t)}
                >
                  <div class={css.cardTitle}>
                    <span
                      class={`${css.priorityDot} ${priorityClass(t.priority)}`}
                    />
                    {t.title}
                  </div>
                  {(t.assignee || (t.tags && t.tags.length > 0)) && (
                    <div class={css.cardMeta}>
                      {t.assignee && (
                        <span
                          class={css.cardAssignee}
                          style={{ background: getAssigneeColor(t.assignee) }}
                          title={t.assignee}
                        >
                          {t.assignee.charAt(0)}
                        </span>
                      )}
                      {t.tags?.map((tag) => (
                        <span key={tag} class={css.cardTag}>
                          {tag}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          </div>
        ))}
      </div>
      <TaskEditor
        visible={editorOpen}
        task={editingTask}
        onClose={() => {
          taskEditorOpen.value = false;
        }}
      />
      {detailTask && <TaskDetail task={detailTask} onClose={handleCloseTask} />}
    </div>
  );
}
