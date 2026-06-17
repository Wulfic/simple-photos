/**
 * App header with navigation, theme toggle, backup-mode indicator,
 * and hamburger menu (mobile). Visible on all authenticated pages.
 */
import { useState, useRef, useEffect } from "react";
import { useLocation } from "react-router-dom";
import { useAppNavigate } from "../hooks/useAppNavigate";
import { useAuthStore } from "../store/auth";
import { useThemeStore } from "../store/theme";
import { useBackupStore } from "../store/backup";
import { clearKey } from "../crypto/crypto";
import { api } from "../api/client";
import { useProcessingStore } from "../store/processing";
import AppIcon from "./AppIcon";
import CastDialog, { CastIcon } from "./CastDialog";
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
  const navigate = useAppNavigate();
  const { pathname } = useLocation();
  const { username, refreshToken, logout: storeLogout, accessToken } = useAuthStore();
  const { theme, toggle: toggleTheme } = useThemeStore();
  const { backupServers, loaded: backupLoaded, setBackupServers, setLoaded: setBackupLoaded } = useBackupStore();

  const hasActivity = useProcessingStore((s) => s.isProcessing);
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const [castOpen, setCastOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  // The processing store is driven by the global progress banners
  // (EncryptionBanner, ConversionBanner, AiBanner, GeoBanner, SavingBanner)
  // mounted in `App.tsx`.  Each banner owns its own task lifecycle and
  // calls `endTask` as soon as its backlog reaches zero, so the avatar
  // spinner stops the moment the underlying server-side work completes.

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
        await api.auth.logout(refreshToken).catch((e) => {
          console.error("Logout API call failed:", e);
        });
      }
    } finally {
      // Wipe all locally cached user data (IndexedDB, Cache API, memory)
      // BEFORE clearing auth state to prevent any flash of stale photos.
      await clearAllUserData().catch((e) => {
        console.error("Failed to clear local user data during logout:", e);
      });
      thumbMemoryCache.clear();
      clearKey();
      storeLogout();
      navigate("/login");
    }
  }

  return (
    <>
    <header className="fixed top-0 left-0 right-0 z-50 bg-surface/80 backdrop-blur-md border-b border-edge shadow-sm dark:bg-gradient-to-r dark:from-gray-900 dark:via-gray-800 dark:to-gray-900 dark:border-white/10 dark:shadow-lg dark:shadow-black/20 [view-transition-name:app-header]">
      {/* Full-bleed: the nav must sit near the screen edges. A centred
          `max-w-screen-2xl mx-auto` cap left large empty gutters between the
          edge and the buttons on wide (Ubuntu) monitors — issue: "too much
          spacing on the sides between screen edge and the buttons". */}
      <div className="w-full px-4 h-14 flex items-center gap-4 min-w-0">
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
          <span className="text-fg font-semibold text-lg tracking-tight hidden sm:inline">
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
                      ? "bg-edge text-fg shadow-inner dark:bg-white/15 dark:text-white"
                      : "text-fg-muted hover:text-fg hover:bg-surface-sunken dark:text-gray-400 dark:hover:text-white dark:hover:bg-white/10"
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
          <div className="flex items-center gap-2 border-l border-edge dark:border-white/10 pl-2 sm:pl-4 ml-1 sm:ml-2 mr-1 shrink-0">
            <div className="relative" ref={dropdownRef}>
              <button
                onClick={() => setDropdownOpen((v) => !v)}
                className="flex items-center gap-2 text-fg-muted hover:text-fg dark:text-gray-400 dark:hover:text-white text-xs transition-colors"
              >
                <div className={`w-6 h-6 rounded-full bg-gradient-to-br from-blue-500 to-purple-600 flex items-center justify-center text-white text-xs font-bold uppercase shrink-0${hasActivity ? " processing-ring" : ""}`}>
                  {username.charAt(0)}
                </div>
                <span className="hidden sm:inline truncate">{username}</span>
              </button>

              {dropdownOpen && (
                // Anchor to the avatar button (the `relative` parent) with
                // `absolute right-0 top-full` rather than `fixed right-4`.
                // Fixed positioning pinned the menu to the viewport's right
                // edge, so on wide / centered layouts (and at non-100% UI
                // scale) it detached from the button and floated off to the
                // side (issue #5). top-full drops it directly below the avatar.
                <div className="absolute right-0 top-full mt-2 w-44 bg-surface rounded-lg shadow-2xl border border-edge py-1" style={{ zIndex: 9999 }}>
                  <button
                    onClick={() => { navigate("/secure-gallery"); setDropdownOpen(false); }}
                    className="w-full text-left px-4 py-2 text-sm text-fg-muted hover:bg-surface-sunken dark:hover:bg-white/10 flex items-center gap-2 transition-colors"
                  >
                    <AppIcon name="locks" />
                    Secure Albums
                  </button>
                  <button
                    onClick={() => { navigate("/settings"); setDropdownOpen(false); }}
                    className="w-full text-left px-4 py-2 text-sm text-fg-muted hover:bg-surface-sunken dark:hover:bg-white/10 flex items-center gap-2 transition-colors"
                  >
                    <AppIcon name="gear" />
                    Settings
                  </button>
                  {isAdmin && (
                  <button
                    onClick={() => { navigate("/diagnostics"); setDropdownOpen(false); }}
                    className="w-full text-left px-4 py-2 text-sm text-fg-muted hover:bg-surface-sunken dark:hover:bg-white/10 flex items-center gap-2 transition-colors"
                  >
                    <AppIcon name="shield" />
                    Diagnostics
                  </button>
                  )}
                  <div className="border-t border-edge my-1" />
                  <button
                    onClick={() => { setCastOpen(true); setDropdownOpen(false); }}
                    className="w-full text-left px-4 py-2 text-sm text-fg-muted hover:bg-surface-sunken dark:hover:bg-white/10 flex items-center gap-2 transition-colors"
                  >
                    <CastIcon className="w-4 h-4" />
                    Cast…
                  </button>
                  <div className="border-t border-edge my-1" />
                  <button
                    onClick={() => { toggleTheme(); setDropdownOpen(false); }}
                    className="w-full text-left px-4 py-2 text-sm text-fg-muted hover:bg-surface-sunken dark:hover:bg-white/10 flex items-center gap-2 transition-colors"
                  >
                    {theme === "dark" ? (
                      <AppIcon name="night" />
                    ) : (
                      <AppIcon name="sun" />
                    )}
                    {theme === "dark" ? "Dark Mode" : "Light Mode"}
                  </button>
                  <div className="border-t border-edge my-1" />
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
    <div className="h-14" />
    <CastDialog open={castOpen} onClose={() => setCastOpen(false)} />
    </>
  );
}
