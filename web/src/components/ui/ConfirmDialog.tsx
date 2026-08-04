/**
 * Shared confirmation prompt — the "are you sure" step, with a real dialog
 * instead of `window.confirm`.
 *
 * The app had two ways of asking: native `confirm()` (secure gallery) and no
 * prompt at all (regular album "Remove"). Neither could say *what would happen*,
 * which is the whole point of asking — and a native confirm cannot be themed,
 * cannot render a destructive-action colour, and is suppressible by the browser.
 *
 * `body` is deliberately a `ReactNode` and deliberately required: a confirm that
 * only restates its own title ("Remove these photos? / Remove") teaches the user
 * to click through it. Say what changes and what does not.
 *
 * `tone="danger"` is for actions that destroy data. Removing a photo from an
 * album is **not** one of them — it un-files the photo, it does not delete it —
 * so that case uses the default tone on purpose. Colouring a reversible action
 * red is how a red button stops meaning anything.
 */
import { useEffect, useRef, type ReactNode } from "react";
import { Modal } from "./Modal";
import { Button } from "./Button";

export interface ConfirmDialogProps {
  /** Short question, e.g. "Remove 3 photos from this album?" */
  title: ReactNode;
  /** What will actually happen. Required — see the module doc. */
  body: ReactNode;
  /** Affirmative label. Default "Yes". */
  confirmLabel?: string;
  /** Negative label. Default "No". */
  cancelLabel?: string;
  /** Red confirm button, for genuinely destructive actions only. */
  tone?: "default" | "danger";
  /** Disables the confirm button and shows `busyLabel` while an action runs. */
  busy?: boolean;
  busyLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  title,
  body,
  confirmLabel = "Yes",
  cancelLabel = "No",
  tone = "default",
  busy = false,
  busyLabel = "Working…",
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  // Focus lands on CANCEL, not confirm: this dialog exists because an action is
  // worth a second look, and a focused confirm turns a stray Enter into the very
  // click the prompt was added to prevent.
  const cancelRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    cancelRef.current?.focus();
  }, []);

  return (
    <Modal
      onClose={busy ? () => {} : onCancel}
      size="sm"
      title={title}
      closeOnBackdrop={!busy}
      closeOnEscape={!busy}
      testId="confirm-dialog"
    >
      <div className="px-4 py-3 text-sm text-fg-muted">{body}</div>
      <div className="flex justify-end gap-2 px-4 pb-4">
        <Button
          ref={cancelRef}
          variant="ghost"
          size="md"
          onClick={onCancel}
          disabled={busy}
          data-testid="confirm-cancel"
        >
          {cancelLabel}
        </Button>
        <Button
          variant={tone === "danger" ? "danger" : "primary"}
          size="md"
          onClick={onConfirm}
          disabled={busy}
          data-testid="confirm-accept"
        >
          {busy ? busyLabel : confirmLabel}
        </Button>
      </div>
    </Modal>
  );
}
