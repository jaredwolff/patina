import { useState, useEffect } from "preact/hooks";
import { personas } from "../state/personas";
import { Modal } from "./Modal";
import * as api from "../api";
import { loadTasks } from "../state/tasks";
import type { Task } from "../types";
import css from "./TasksView.module.css";

interface TaskEditorProps {
  visible: boolean;
  task: Task | null;
  onClose: () => void;
}

export function TaskEditor({ visible, task, onClose }: TaskEditorProps) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [priority, setPriority] = useState("medium");
  const [assignee, setAssignee] = useState("");
  const [tags, setTags] = useState("");

  const personaList = personas.value;

  useEffect(() => {
    if (visible) {
      if (task) {
        setTitle(task.title);
        setDescription(task.description || "");
        setPriority(task.priority || "medium");
        setAssignee(task.assignee || "");
        setTags((task.tags || []).join(", "));
      } else {
        setTitle("");
        setDescription("");
        setPriority("medium");
        setAssignee("");
        setTags("");
      }
    }
  }, [visible, task]);

  async function handleSubmit(e: Event) {
    e.preventDefault();
    const tagList = tags
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);

    try {
      if (task) {
        await api.updateTask(task.id, {
          title,
          description,
          priority: priority as Task["priority"],
          tags: tagList,
        });
        const newAssignee = assignee || null;
        const oldAssignee = task.assignee || null;
        if (newAssignee !== oldAssignee) {
          await api.assignTask(task.id, newAssignee);
        }
      } else {
        await api.createTask({
          title,
          description,
          priority: priority as Task["priority"],
          assignee: assignee || null,
          tags: tagList,
        });
      }
      await loadTasks();
      onClose();
    } catch (err) {
      console.error("Task save failed:", err);
    }
  }

  async function handleDelete() {
    if (!task || !confirm("Delete this task?")) return;
    await api.deleteTask(task.id);
    await loadTasks();
    onClose();
  }

  return (
    <Modal visible={visible} onClose={onClose} wide>
      <h3>{task ? "Edit Task" : "New Task"}</h3>
      <form class={css.editorForm} onSubmit={handleSubmit}>
        <label>
          Title
          <input
            type="text"
            required
            placeholder="Task title"
            value={title}
            onInput={(e) => setTitle((e.target as HTMLInputElement).value)}
          />
        </label>
        <label>
          Description
          <textarea
            rows={3}
            placeholder="Markdown description..."
            value={description}
            onInput={(e) => setDescription((e.target as HTMLTextAreaElement).value)}
          />
        </label>
        <label>
          Priority
          <select value={priority} onChange={(e) => setPriority((e.target as HTMLSelectElement).value)}>
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="high">High</option>
            <option value="urgent">Urgent</option>
          </select>
        </label>
        <label>
          Assignee
          <select value={assignee} onChange={(e) => setAssignee((e.target as HTMLSelectElement).value)}>
            <option value="">Unassigned</option>
            {personaList.map((p) => (
              <option key={p.key} value={p.key}>{p.name}</option>
            ))}
          </select>
        </label>
        <label>
          Tags (comma-separated)
          <input
            type="text"
            placeholder="e.g. bug, frontend"
            value={tags}
            onInput={(e) => setTags((e.target as HTMLInputElement).value)}
          />
        </label>
        <div class="modal-actions">
          {task && (
            <button type="button" class="btn-danger" style={{ marginRight: "auto" }} onClick={handleDelete}>
              Delete
            </button>
          )}
          <button type="button" class="btn-secondary" style={{ width: "auto" }} onClick={onClose}>
            Cancel
          </button>
          <button type="submit" class="btn-primary">Save</button>
        </div>
      </form>
    </Modal>
  );
}
