/** Modal prompt confirming unsaved edit changes before navigating away. */
import { Modal } from "../ui";

interface LeavePromptProps {
  show: boolean;
  onCancel: () => void;
  onDiscard: () => void;
  onSave: () => void;
}

export default function LeavePrompt({ show, onCancel, onDiscard, onSave }: LeavePromptProps) {
  if (!show) return null;

  return (
    <Modal onClose={onCancel} size="sm">
      <div className="p-6 space-y-4">
        <h3 className="text-fg text-lg font-semibold">Unsaved Changes</h3>
        <p className="text-fg-muted text-sm">You have unsaved edits. Would you like to save or discard them?</p>
        <div className="flex gap-3 justify-end">
          <button onClick={onCancel} className="btn btn-secondary btn-md">
            Cancel
          </button>
          <button onClick={onDiscard} className="btn btn-danger btn-md">
            Discard
          </button>
          <button onClick={onSave} className="btn btn-primary btn-md">
            Save
          </button>
        </div>
      </div>
    </Modal>
  );
}
