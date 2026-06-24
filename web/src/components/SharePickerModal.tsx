/**
 * SharePickerModal — pick a user to share an album with.
 *
 * Consolidates the three byte-for-byte-identical inline "share user picker"
 * overlays that previously lived in Albums, SharedAlbumDetail and the regular
 * album view. Built on the shared <Modal> primitive.
 */
import { Modal } from "./ui";

export interface SharePickerUser {
  id: string;
  username: string;
}

interface SharePickerModalProps {
  title: string;
  users: SharePickerUser[];
  onPick: (userId: string) => void;
  onClose: () => void;
  emptyText?: string;
}

export default function SharePickerModal({
  title,
  users,
  onPick,
  onClose,
  emptyText = "No users found",
}: SharePickerModalProps) {
  return (
    <Modal onClose={onClose} size="sm" panelClassName="p-6">
      <h3 className="text-lg font-semibold mb-4">{title}</h3>
      <div className="space-y-2 max-h-64 overflow-y-auto">
        {users.map((u) => (
          <button
            key={u.id}
            onClick={() => onPick(u.id)}
            className="w-full text-left px-3 py-2 rounded-md hover:bg-surface-sunken dark:hover:bg-white/10 text-sm flex items-center gap-2"
          >
            <svg
              className="w-5 h-5 text-fg-muted"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={1.5}
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M15.75 6a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0zM4.501 20.118a7.5 7.5 0 0114.998 0A17.933 17.933 0 0112 21.75c-2.676 0-5.216-.584-7.499-1.632z"
              />
            </svg>
            {u.username}
          </button>
        ))}
        {users.length === 0 && (
          <p className="text-fg-muted text-sm text-center py-4">{emptyText}</p>
        )}
      </div>
      <button
        onClick={onClose}
        className="mt-4 w-full py-2 text-sm text-fg-muted hover:bg-surface-sunken dark:hover:bg-white/10 rounded-md"
      >
        Cancel
      </button>
    </Modal>
  );
}
