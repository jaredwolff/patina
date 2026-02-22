import { personas, loadPersonas } from "../state/personas";
import * as api from "../api";
import { Modal } from "./Modal";
import type { Persona } from "../types";
import css from "./PersonaManager.module.css";

interface PersonaManagerProps {
  visible: boolean;
  onClose: () => void;
  onEdit: (persona: Persona | null) => void;
}

export function PersonaManager({
  visible,
  onClose,
  onEdit,
}: PersonaManagerProps) {
  const personaList = personas.value;

  async function handleDelete(key: string, name: string) {
    if (!confirm(`Delete persona "${name}"?`)) return;
    await api.deletePersona(key);
    await loadPersonas();
  }

  return (
    <Modal visible={visible} onClose={onClose} wide>
      <div class="modal-header">
        <h3>Manage Personas</h3>
        <button class="btn-text" onClick={onClose}>
          &times;
        </button>
      </div>
      <div class={css.list}>
        {personaList.map((p) => (
          <div key={p.key} class={css.item}>
            <div class={css.info}>
              <div class={css.name}>{p.name}</div>
              <div class={css.desc}>{p.description || p.key}</div>
            </div>
            <div class={css.actions}>
              <button
                class="btn-text"
                onClick={() => {
                  onClose();
                  onEdit(p);
                }}
              >
                Edit
              </button>
              <button
                class="btn-danger"
                onClick={() => handleDelete(p.key, p.name)}
              >
                Delete
              </button>
            </div>
          </div>
        ))}
        {personaList.length === 0 && (
          <div class={css.empty}>
            No personas yet. Create one to get started.
          </div>
        )}
      </div>
      <button
        class="btn-primary"
        onClick={() => {
          onClose();
          onEdit(null);
        }}
      >
        + New Persona
      </button>
    </Modal>
  );
}
