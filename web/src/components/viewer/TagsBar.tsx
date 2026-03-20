/**
 * TagsBar — tags are stored in encrypted metadata.
 * This component is a no-op stub kept for API compatibility.
 * TODO: Remove this component and its usages entirely.
 */

interface TagsBarProps {
  show: boolean;
  tags: string[];
  showTagInput: boolean;
  tagInput: string;
  setTagInput: (v: string) => void;
  setShowTagInput: (v: boolean) => void;
  tagSuggestions: string[];
  onAddTag: () => void;
  onRemoveTag: (tag: string) => void;
  tagInputRef: React.RefObject<HTMLInputElement>;
}

export default function TagsBar(_props: TagsBarProps) {
  return null;
}
