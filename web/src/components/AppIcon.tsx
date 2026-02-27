/**
 * Unified icon component that renders custom PNG icons from /icons/.
 * Icons are black-on-transparent PNGs; dark mode uses CSS invert.
 *
 * Available icons:
 *   arrow, back-arrow, download, floppy-disk, folder, gear, home,
 *   image, lock, locks, magnify-glass, menu, night, option, reload,
 *   right-arrow, shared, shield, star, suffle, sun, trashcan, upload
 */

export type IconName =
  | "arrow"
  | "back-arrow"
  | "download"
  | "floppy-disk"
  | "folder"
  | "gear"
  | "home"
  | "image"
  | "lock"
  | "locks"
  | "magnify-glass"
  | "menu"
  | "night"
  | "option"
  | "reload"
  | "right-arrow"
  | "shared"
  | "shield"
  | "star"
  | "suffle"
  | "sun"
  | "trashcan"
  | "upload";

interface AppIconProps {
  /** Icon filename without extension */
  name: IconName;
  /** Tailwind size classes (default: "w-4 h-4") */
  size?: string;
  /** Additional CSS classes */
  className?: string;
  /** Alt text for accessibility */
  alt?: string;
  /** Whether to apply dark-mode inversion (default: true) */
  themed?: boolean;
}

export default function AppIcon({
  name,
  size = "w-4 h-4",
  className = "",
  alt = "",
  themed = true,
}: AppIconProps) {
  return (
    <img
      src={`/icons/${name}.png`}
      alt={alt}
      className={`${size} shrink-0 object-contain ${themed ? "dark:invert" : ""} ${className}`.trim()}
      draggable={false}
    />
  );
}
