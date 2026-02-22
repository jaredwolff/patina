import type { Persona } from "../types";
import { Modal } from "./Modal";
import css from "./PersonaPicker.module.css";

interface PersonaPickerProps {
  visible: boolean;
  personas: Persona[];
  onSelect: (key: string | null) => void;
  onCancel: () => void;
}

export function PersonaPicker({
  visible,
  personas,
  onSelect,
  onCancel,
}: PersonaPickerProps) {
  return (
    <Modal visible={visible} onClose={onCancel}>
      <h3>Choose a Persona</h3>
      <div class={css.list}>
        {personas.map((p) => (
          <div
            key={p.key}
            class={css.card}
            onClick={() => onSelect(p.key)}
          >
            <div class={css.name}>{p.name}</div>
            {p.description && <div class={css.desc}>{p.description}</div>}
            {p.modelTier && p.modelTier !== "default" && (
              <div class={css.tier}>Model: {p.modelTier}</div>
            )}
          </div>
        ))}
      </div>
      <button class="btn-secondary" onClick={() => onSelect(null)}>
        No Persona (default)
      </button>
      <button class="btn-text" onClick={onCancel}>
        Cancel
      </button>
    </Modal>
  );
}
