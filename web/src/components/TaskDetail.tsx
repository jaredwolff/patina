import { useState, useEffect, useRef } from "preact/hooks";
import { personas } from "../state/personas";
import {
  activeTaskId,
  taskMessages,
  taskStreamingText,
  taskIsStreaming,
  send,
} from "../state/websocket";
import { loadTasks } from "../state/tasks";
import { renderMarkdown } from "../lib/markdown";
import * as api from "../api";
import type { Task } from "../types";
import css from "./TasksView.module.css";

interface TaskDetailProps {
  task: Task;
  onClose: () => void;
}

export function TaskDetail({ task, onClose }: TaskDetailProps) {
  const [title, setTitle] = useState(task.title);
  const [status, setStatus] = useState(task.status);
  const [priority, setPriority] = useState(task.priority);
  const [assignee, setAssignee] = useState(task.assignee || "");
  const [tags, setTags] = useState((task.tags || []).join(", "));
  const [descriptionText, setDescriptionText] = useState(
    task.description || "",
  );
  const [editingDesc, setEditingDesc] = useState(false);
  const messagesRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const personaList = personas.value;
  const msgs = taskMessages.value;
  const streaming = taskStreamingText.value;
  const isStreamActive = taskIsStreaming.value;

  // Original values to detect changes
  const origStatus = useRef(task.status);
  const origAssignee = useRef(task.assignee || "");

  useEffect(() => {
    activeTaskId.value = task.id;
    taskMessages.value = [];
    taskStreamingText.value = "";
    taskIsStreaming.value = false;

    send({ type: "get_task_history", chatId: task.id });

    return () => {
      activeTaskId.value = null;
    };
  }, [task.id]);

  useEffect(() => {
    if (messagesRef.current) {
      messagesRef.current.scrollTop = messagesRef.current.scrollHeight;
    }
  }, [msgs, streaming]);

  async function saveMeta() {
    const tagList = tags
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
    const promises: Promise<void>[] = [];

    promises.push(
      api.updateTask(task.id, {
        title,
        description: descriptionText,
        priority,
        tags: tagList,
      }),
    );

    if (status !== origStatus.current) {
      promises.push(
        api.moveTask(task.id, status).then(() => {
          origStatus.current = status;
        }),
      );
    }

    if (assignee !== origAssignee.current) {
      promises.push(
        api.assignTask(task.id, assignee || null).then(() => {
          origAssignee.current = assignee;
        }),
      );
    }

    await Promise.all(promises).catch((err) =>
      console.error("Save failed:", err),
    );
  }

  function handleClose() {
    saveMeta().then(() => {
      loadTasks();
      onClose();
    });
  }

  async function handleDelete() {
    if (!confirm("Delete this task?")) return;
    await api.deleteTask(task.id);
    await loadTasks();
    onClose();
  }

  function handleSendMessage(e: Event) {
    e.preventDefault();
    const el = inputRef.current;
    if (!el) return;
    const text = el.value.trim();
    if (!text) return;

    taskMessages.value = [
      ...taskMessages.value,
      { role: "user", content: text },
    ];
    el.value = "";

    send({ type: "task_message", chatId: task.id, content: text });
  }

  function handleCopyId() {
    const sessionKey = "task:" + task.id;
    navigator.clipboard.writeText(sessionKey).catch(() => {});
  }

  return (
    <div
      class="modal"
      onClick={(e) => {
        if (e.target === e.currentTarget) handleClose();
      }}
    >
      <div class={css.detailContent}>
        <div class={css.detailHeader}>
          <input
            type="text"
            class={css.detailTitleInput}
            placeholder="Task title"
            value={title}
            onInput={(e) => setTitle((e.target as HTMLInputElement).value)}
            onChange={() => saveMeta()}
          />
          <span class={css.detailId} onClick={handleCopyId}>
            #{task.id}
          </span>
          <button
            class="btn-danger btn-sm"
            title="Delete task"
            onClick={handleDelete}
          >
            Delete
          </button>
          <button class="btn-text" title="Close" onClick={handleClose}>
            &times;
          </button>
        </div>
        <div class={css.detailBody}>
          <div class={css.detailLeft}>
            <div class={css.detailMeta}>
              <label>
                Status
                <select
                  value={status}
                  onChange={(e) => {
                    setStatus(
                      (e.target as HTMLSelectElement).value as Task["status"],
                    );
                    saveMeta();
                  }}
                >
                  <option value="backlog">Backlog</option>
                  <option value="todo">Todo</option>
                  <option value="in_progress">In Progress</option>
                  <option value="done">Done</option>
                </select>
              </label>
              <label>
                Priority
                <select
                  value={priority}
                  onChange={(e) => {
                    setPriority(
                      (e.target as HTMLSelectElement).value as Task["priority"],
                    );
                    saveMeta();
                  }}
                >
                  <option value="low">Low</option>
                  <option value="medium">Medium</option>
                  <option value="high">High</option>
                  <option value="urgent">Urgent</option>
                </select>
              </label>
              <label>
                Assignee
                <select
                  value={assignee}
                  onChange={(e) => {
                    setAssignee((e.target as HTMLSelectElement).value);
                    saveMeta();
                  }}
                >
                  <option value="">Unassigned</option>
                  {personaList.map((p) => (
                    <option key={p.key} value={p.key}>
                      {p.name}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                Tags
                <input
                  type="text"
                  placeholder="comma-separated"
                  value={tags}
                  onInput={(e) => setTags((e.target as HTMLInputElement).value)}
                  onChange={() => saveMeta()}
                />
              </label>
            </div>
            <div class={css.sectionLabel}>Description</div>
            {editingDesc ? (
              <textarea
                class={css.descriptionEdit}
                placeholder="Description (markdown supported)..."
                value={descriptionText}
                onInput={(e) =>
                  setDescriptionText((e.target as HTMLTextAreaElement).value)
                }
                onBlur={() => {
                  setEditingDesc(false);
                  saveMeta();
                }}
                autoFocus
              />
            ) : (
              <div
                class={css.description}
                onClick={() => setEditingDesc(true)}
                dangerouslySetInnerHTML={{
                  __html: descriptionText
                    ? renderMarkdown(descriptionText)
                    : '<span class="' +
                      css.descriptionPlaceholder +
                      '">Click to add a description...</span>',
                }}
              />
            )}
          </div>
          <div class={css.detailRight}>
            <div class={css.taskMessages} ref={messagesRef}>
              {msgs.length === 0 && !isStreamActive && (
                <div class={css.taskMessagesEmpty}>
                  No messages yet. Start a conversation about this task.
                </div>
              )}
              {msgs.map((msg, i) => (
                <div key={i} class={`message ${msg.role}`}>
                  {msg.role === "assistant" ? (
                    <span
                      dangerouslySetInnerHTML={{
                        __html: renderMarkdown(msg.content),
                      }}
                    />
                  ) : (
                    msg.content
                  )}
                </div>
              ))}
              {isStreamActive && streaming && (
                <div
                  class="message assistant"
                  dangerouslySetInnerHTML={{
                    __html: renderMarkdown(streaming),
                  }}
                />
              )}
            </div>
            <form class={css.taskInputForm} onSubmit={handleSendMessage}>
              <textarea
                ref={inputRef}
                placeholder="Discuss this task..."
                rows={1}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    handleSendMessage(e);
                  }
                }}
                onInput={(e) => {
                  const el = e.target as HTMLTextAreaElement;
                  el.style.height = "auto";
                  el.style.height = Math.min(el.scrollHeight, 120) + "px";
                }}
              />
              <button type="submit">Send</button>
            </form>
          </div>
        </div>
      </div>
    </div>
  );
}
