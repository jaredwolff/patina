import type { ComponentChildren } from "preact";

interface ModalProps {
  visible: boolean;
  onClose: () => void;
  wide?: boolean;
  children: ComponentChildren;
}

export function Modal({ visible, onClose, wide, children }: ModalProps) {
  if (!visible) return null;

  return (
    <div class="modal" onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div class={`modal-content${wide ? " modal-wide" : ""}`}>
        {children}
      </div>
    </div>
  );
}
