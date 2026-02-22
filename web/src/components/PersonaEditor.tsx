import { useState, useEffect } from "preact/hooks";
import { modelTiers, loadModelTiers, PRESET_COLORS, loadPersonas } from "../state/personas";
import * as api from "../api";
import { Modal } from "./Modal";
import type { Persona } from "../types";
import css from "./PersonaEditor.module.css";

interface PersonaEditorProps {
  visible: boolean;
  persona: Persona | null;
  onClose: () => void;
}

export function PersonaEditor({ visible, persona, onClose }: PersonaEditorProps) {
  const [key, setKey] = useState("");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [preamble, setPreamble] = useState("");
  const [color, setColor] = useState("");
  const [tier, setTier] = useState("default");
  const [generating, setGenerating] = useState(false);

  const tiers = modelTiers.value;

  useEffect(() => {
    if (visible) {
      loadModelTiers();
      if (persona) {
        setKey(persona.key);
        setName(persona.name);
        setDescription(persona.description || "");
        setPreamble(persona.preamble || "");
        setColor(persona.color || "");
        setTier(persona.modelTier || "default");
      } else {
        setKey("");
        setName("");
        setDescription("");
        setPreamble("");
        setColor("");
        setTier("default");
      }
    }
  }, [visible, persona]);

  async function handleSubmit(e: Event) {
    e.preventDefault();
    const data = {
      key,
      name,
      description,
      preamble,
      modelTier: tier,
      color,
    };

    try {
      if (persona) {
        await api.updatePersona(persona.key, data);
      } else {
        await api.createPersona(data);
      }
      await loadPersonas();
      onClose();
    } catch (err: unknown) {
      alert("Error: " + (err instanceof Error ? err.message : String(err)));
    }
  }

  async function handleGenerate() {
    if (!name.trim()) {
      alert("Enter a persona name first.");
      return;
    }
    setGenerating(true);
    try {
      const data = await api.generatePersonaPrompt({ name: name.trim(), description: description.trim() });
      if (data.preamble) {
        setPreamble(data.preamble);
      }
    } catch (err: unknown) {
      alert("Generation failed: " + (err instanceof Error ? err.message : String(err)));
    } finally {
      setGenerating(false);
    }
  }

  return (
    <Modal visible={visible} onClose={onClose} wide>
      <h3>{persona ? "Edit Persona" : "New Persona"}</h3>
      <form class={css.form} onSubmit={handleSubmit}>
        <label>
          Key (unique ID)
          <input
            type="text"
            required
            placeholder="e.g. coder"
            value={key}
            disabled={!!persona}
            onInput={(e) => setKey((e.target as HTMLInputElement).value)}
          />
        </label>
        <label>
          Name
          <input
            type="text"
            required
            placeholder="e.g. Code Assistant"
            value={name}
            onInput={(e) => setName((e.target as HTMLInputElement).value)}
          />
        </label>
        <label>
          Description
          <input
            type="text"
            placeholder="Short description"
            value={description}
            onInput={(e) => setDescription((e.target as HTMLInputElement).value)}
          />
        </label>
        <label>
          Color
          <div class="color-swatches">
            {PRESET_COLORS.map((c) => (
              <div
                key={c}
                class={`color-swatch${c === color ? " selected" : ""}`}
                style={{ background: c }}
                onClick={() => setColor(c)}
              />
            ))}
          </div>
        </label>
        <label>
          <span class={css.labelWithAction}>
            System Prompt
            <button
              type="button"
              class="btn-inline"
              disabled={generating}
              onClick={handleGenerate}
            >
              {generating ? "Generating..." : "Generate"}
            </button>
          </span>
          <textarea
            rows={5}
            placeholder="Custom system prompt..."
            value={preamble}
            onInput={(e) => setPreamble((e.target as HTMLTextAreaElement).value)}
          />
        </label>
        <label>
          Model Tier
          <select
            value={tier}
            onChange={(e) => setTier((e.target as HTMLSelectElement).value)}
          >
            {!tiers.includes("default") && <option value="default">default</option>}
            {tiers.map((t) => (
              <option key={t} value={t}>{t}</option>
            ))}
          </select>
        </label>
        <div class="modal-actions">
          <button type="button" class="btn-secondary" style={{ width: "auto" }} onClick={onClose}>
            Cancel
          </button>
          <button type="submit" class="btn-primary">Save</button>
        </div>
      </form>
    </Modal>
  );
}
