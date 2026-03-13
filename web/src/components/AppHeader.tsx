/**
 * App header with navigation, theme toggle, backup-mode indicator,
 * and hamburger menu (mobile). Visible on all authenticated pages.
 */
import { useState, useRef, useEffect } from "react";
import { useNavigate, useLocation } from "react-router-dom";
import { useAuthStore } from "../store/auth";
import { useThemeStore } from "../store/theme";
import { useBackupStore } from "../store/backup";
import { useProcessingStore } from "../store/processing";
import { clearKey } from "../crypto/crypto";
import { api } from "../api/client";
import AppIcon from "./AppIcon";
import { clearAllUserData } from "../db";
import { thumbMemoryCache } from "../utils/gallery";

interface NavItem {
  label: string;
  path: string;
  /** Optional icon — pass a className-based icon or small SVG */
  icon?: React.ReactNode;
}

const NAV_ITEMS: NavItem[] = [
  {
    label: "Gallery",
    path: "/gallery",
    icon: <AppIcon name="image" />,
  },
  {
    label: "Albums",
    path: "/albums",
    icon: <AppIcon name="folder" />,
  },
  {
    label: "Search",
    path: "/search",
    icon: <AppIcon name="magnify-glass" />,
  },
  {
    label: "Trash",
    path: "/trash",
    icon: <AppIcon name="trashcan" />,
  },
];

/**
 * Shared application header with consistent navigation across all pages.
 *
 * Features:
 * - Glassmorphic dark header with subtle gradient
 * - Logo + app name on the left
 * - Consistent nav links with active state highlighting
 * - Optional right-side action buttons passed as children
 */
export default function AppHeader({
  children,
}: {
  /** Optional action buttons (Upload, New Album, etc.) rendered on the right */
  children?: React.ReactNode;
}) {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const { username, refreshToken, logout: storeLogout, accessToken } = useAuthStore();
  const { theme, toggle: toggleTheme } = useThemeStore();
  const { backupServers, loaded: backupLoaded, setBackupServers, setLoaded: setBackupLoaded } = useBackupStore();
  const { isProcessing, activeLabel } = useProcessingStore();
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  // Check admin status from JWT
  const isAdmin = (() => {
    if (!accessToken) return false;
    try {
      const payload = JSON.parse(atob(accessToken.split(".")[1]));
      return payload.role === "admin";
    } catch {
      return false;
    }
  })();

  // Load backup servers on mount (only once)
  useEffect(() => {
    if (backupLoaded) return;
    api.backup.listServers()
      .then((res) => setBackupServers(res.servers))
      .catch(() => {})
      .finally(() => setBackupLoaded(true));
  }, [backupLoaded, setBackupServers, setBackupLoaded]);

  // Close dropdown when clicking outside
  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setDropdownOpen(false);
      }
    }
    if (dropdownOpen) {
      document.addEventListener("mousedown", handleClickOutside);
    }
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [dropdownOpen]);

  async function handleLogout() {
    try {
      if (refreshToken) {
        await api.auth.logout(refreshToken).catch(() => {});
      }
    } finally {
      // Wipe all locally cached user data (IndexedDB, Cache API, memory)
      // BEFORE clearing auth state to prevent any flash of stale photos.
      await clearAllUserData().catch(() => {});
      thumbMemoryCache.clear();
      clearKey();
      storeLogout();
      navigate("/login");
    }
  }

  return (
    <header className="sticky top-0 z-50 bg-white border-b border-gray-200 shadow-sm dark:bg-gradient-to-r dark:from-gray-900 dark:via-gray-800 dark:to-gray-900 dark:border-white/10 dark:shadow-lg dark:shadow-black/20">
      <div className="max-w-screen-2xl mx-auto px-4 h-14 flex items-center gap-4 min-w-0">
        {/* ── Logo + Brand ────────────────────────────────────────────── */}
        <button
          onClick={() => navigate("/gallery")}
          className="flex items-center gap-2.5 shrink-0 group"
        >
          <img
            src="/logo.png"
            alt="Simple Photos"
            className="w-8 h-8 rounded-md shadow-sm group-hover:shadow-md transition-shadow"
          />
          <span className="text-gray-900 dark:text-white font-semibold text-lg tracking-tight hidden sm:inline">
            Simple Photos
          </span>
        </button>

        {/* ── Navigation ──────────────────────────────────────────────── */}
        <nav className="flex items-center gap-0.5 sm:gap-1 ml-1 sm:ml-2">
          {NAV_ITEMS.map((item) => {
            const isActive =
              pathname === item.path ||
              (item.path === "/albums" && pathname.startsWith("/albums/"));

            return (
              <button
                key={item.path}
                onClick={() => navigate(item.path)}
                className={`
                  flex items-center gap-1.5 px-2 sm:px-3 py-1.5 rounded-md text-sm font-medium
                  transition-all duration-200
                  ${
                    isActive
                      ? "bg-gray-200 text-gray-900 dark:bg-white/15 dark:text-white shadow-inner"
                      : "text-gray-500 hover:text-gray-900 hover:bg-gray-100 dark:text-gray-400 dark:hover:text-white dark:hover:bg-white/10"
                  }
                `}
              >
                {item.icon}
                <span className="hidden md:inline">{item.label}</span>
              </button>
            );
          })}
        </nav>

        {/* ── Page-specific actions (e.g. upload +) ─────────────────── */}
        {children}


        {/* ── Spacer ──────────────────────────────────────────────────── */}
        <div className="flex-1" />

        {/* ── Activity indicator + User dropdown ──────────────────────── */}
        {username && (
          <div className="flex items-center gap-2 border-l border-gray-200 dark:border-white/10 pl-2 sm:pl-4 ml-1 sm:ml-2 mr-1 shrink-0">
            <div className="relative" ref={dropdownRef}>
              <button
                onClick={() => setDropdownOpen((v) => !v)}
                className="flex items-center gap-2 text-gray-500 hover:text-gray-900 dark:text-gray-400 dark:hover:text-white text-xs transition-colors"
              >
                <div className={`w-6 h-6 rounded-full bg-gradient-to-br from-blue-500 to-purple-600 flex items-center justify-center text-white text-xs font-bold uppercase shrink-0${isProcessing ? " processing-ring" : ""}`}>
                  {username.charAt(0)}
                </div>
                <span className="hidden sm:inline truncate">{username}</span>
              </button>

              {dropdownOpen && (
                <div className="fixed right-4 mt-2 w-44 bg-white dark:bg-gray-800 rounded-lg shadow-2xl border border-gray-200 dark:border-gray-700 py-1" style={{ top: '3.5rem', zIndex: 9999 }}>
                  <button
                    onClick={() => { navigate("/secure-gallery"); setDropdownOpen(false); }}
                    className="w-full text-left px-4 py-2 text-sm text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 transition-colors"
                  >
                    <AppIcon name="locks" />
                    Secure Albums
                  </button>
                  <button
                    onClick={() => { navigate("/settings"); setDropdownOpen(false); }}
                    className="w-full text-left px-4 py-2 text-sm text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 transition-colors"
                  >
                    <AppIcon name="gear" />
                    Settings
                  </button>
                  {isAdmin && (
                  <button
                    onClick={() => { navigate("/diagnostics"); setDropdownOpen(false); }}
                    className="w-full text-left px-4 py-2 text-sm text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 transition-colors"
                  >
                    <AppIcon name="shield" />
                    Diagnostics
                  </button>
                  )}
                  <div className="border-t border-gray-200 dark:border-gray-700 my-1" />
                  <button
                    onClick={() => { toggleTheme(); setDropdownOpen(false); }}
                    className="w-full text-left px-4 py-2 text-sm text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 transition-colors"
                  >
                    {theme === "dark" ? (
                      <AppIcon name="night" />
                    ) : (
                      <AppIcon name="sun" />
                    )}
                    {theme === "dark" ? "Dark Mode" : "Light Mode"}
                  </button>
                  <div className="border-t border-gray-200 dark:border-gray-700 my-1" />
                  <button
                    onClick={() => { handleLogout(); setDropdownOpen(false); }}
                    className="w-full text-left px-4 py-2 text-sm text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-900/30 flex items-center gap-2 transition-colors"
                  >
                    Sign Out
                  </button>
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </header>
  );
}
