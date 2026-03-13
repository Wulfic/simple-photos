/** Modal prompt confirming unsaved edit changes before navigating away. */
interface LeavePromptProps {
  show: boolean;
  onCancel: () => void;
  onDiscard: () => void;
  onSave: () => void;
}

export default function LeavePrompt({ show, onCancel, onDiscard, onSave }: LeavePromptProps) {
  if (!show) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70">
      <div className="bg-gray-800 rounded-xl p-6 max-w-sm w-full mx-4 space-y-4">
        <h3 className="text-white text-lg font-semibold">Unsaved Changes</h3>
        <p className="text-gray-300 text-sm">You have unsaved edits. Would you like to save or discard them?</p>
        <div className="flex gap-3 justify-end">
          <button
            onClick={onCancel}
            className="px-4 py-2 bg-gray-700 text-white rounded-lg text-sm font-medium hover:bg-gray-600 transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={onDiscard}
            className="px-4 py-2 bg-red-600 text-white rounded-lg text-sm font-medium hover:bg-red-700 transition-colors"
          >
            Discard
          </button>
          <button
            onClick={onSave}
            className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm font-medium hover:bg-blue-700 transition-colors"
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
