import { useThemeStore } from "../store/theme";
import AppIcon from "./AppIcon";

/**
 * Standalone theme toggle button for pages that don't use AppHeader
 * (e.g. Login, Register, Welcome, Setup).
 * Renders a small floating button in the top-right corner.
 */
export default function ThemeToggle() {
  const { theme, toggle } = useThemeStore();

  return (
    <button
      onClick={toggle}
      className="fixed top-4 right-4 z-50 p-2 rounded-full bg-white/80 dark:bg-gray-800/80 shadow-md hover:shadow-lg border border-gray-200 dark:border-gray-700 transition-all backdrop-blur-sm"
      title={theme === "light" ? "Switch to dark mode" : "Switch to light mode"}
    >
      {theme === "light" ? (
        <AppIcon name="night" size="w-5 h-5" className="text-gray-600 dark:text-gray-400" />
      ) : (
        <AppIcon name="sun" size="w-5 h-5" className="text-gray-600 dark:text-gray-400" />
      )}
    </button>
  );
}
