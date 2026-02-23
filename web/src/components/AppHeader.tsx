import { useState, useRef, useEffect } from "react";
import { useNavigate, useLocation } from "react-router-dom";
import { useAuthStore } from "../store/auth";
import { useThemeStore } from "../store/theme";
import { useBackupStore } from "../store/backup";
import { useProcessingStore } from "../store/processing";
import { clearKey } from "../crypto/crypto";
import { api } from "../api/client";

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
    icon: (
      <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M2.25 15.75l5.159-5.159a2.25 2.25 0 013.182 0l5.159 5.159m-1.5-1.5l1.409-1.409a2.25 2.25 0 013.182 0l2.909 2.909M3.75 21h16.5A2.25 2.25 0 0022.5 18.75V5.25A2.25 2.25 0 0020.25 3H3.75A2.25 2.25 0 001.5 5.25v13.5A2.25 2.25 0 003.75 21z" />
      </svg>
    ),
  },
  {
    label: "Albums",
    path: "/albums",
    icon: (
      <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M2.25 12.75V12A2.25 2.25 0 014.5 9.75h15A2.25 2.25 0 0121.75 12v.75m-8.69-6.44l-2.12-2.12a1.5 1.5 0 00-1.061-.44H4.5A2.25 2.25 0 002.25 6v12a2.25 2.25 0 002.25 2.25h15A2.25 2.25 0 0021.75 18V9a2.25 2.25 0 00-2.25-2.25h-5.379a1.5 1.5 0 01-1.06-.44z" />
      </svg>
    ),
  },
  {
    label: "Import",
    path: "/import",
    icon: (
      <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5" />
      </svg>
    ),
  },
  {
    label: "Trash",
    path: "/trash",
    icon: (
      <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0" />
      </svg>
    ),
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
  const { username, refreshToken, logout: storeLogout } = useAuthStore();
  const { theme, toggle: toggleTheme } = useThemeStore();
  const { viewMode, toggleViewMode, backupServers, loaded: backupLoaded, setBackupServers, setLoaded: setBackupLoaded } = useBackupStore();
  const { isProcessing } = useProcessingStore();
  const hasBackup = backupServers.length > 0;
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

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
      clearKey();
      storeLogout();
      navigate("/login");
    }
  }

  return (
    <header className="sticky top-0 z-50 bg-gradient-to-r from-gray-900 via-gray-800 to-gray-900 border-b border-white/10 shadow-lg shadow-black/20">
      <div className="max-w-screen-2xl mx-auto px-4 h-14 flex items-center gap-4">
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
          <span className="text-white font-semibold text-lg tracking-tight hidden sm:inline">
            Simple Photos
          </span>
        </button>

        {/* ── Navigation ──────────────────────────────────────────────── */}
        <nav className="flex items-center gap-1 ml-2">
          {NAV_ITEMS.map((item) => {
            const isActive =
              pathname === item.path ||
              (item.path === "/albums" && pathname.startsWith("/albums/"));

            return (
              <button
                key={item.path}
                onClick={() => navigate(item.path)}
                className={`
                  flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium
                  transition-all duration-200
                  ${
                    isActive
                      ? "bg-white/15 text-white shadow-inner"
                      : "text-gray-400 hover:text-white hover:bg-white/10"
                  }
                `}
              >
                {item.icon}
                <span className="hidden md:inline">{item.label}</span>
              </button>
            );
          })}
        </nav>

        {/* ── Backup / Main toggle ────────────────────────────────────── */}
        <div
          className={`flex items-center gap-2 ml-3 pl-3 border-l border-white/10 ${
            !hasBackup ? "opacity-40 pointer-events-none" : ""
          }`}
          title={
            !hasBackup
              ? "No backup server configured"
              : viewMode === "main"
              ? "Switch to backup view"
              : "Switch to main view"
          }
        >
          <span
            className={`text-xs font-medium transition-colors ${
              viewMode === "main" ? "text-white" : "text-gray-500"
            }`}
          >
            Main
          </span>
          <button
            onClick={toggleViewMode}
            disabled={!hasBackup}
            className={`
              relative w-9 h-5 rounded-full transition-colors duration-200
              focus:outline-none focus:ring-2 focus:ring-blue-500/50
              ${viewMode === "backup" ? "bg-blue-600" : "bg-gray-600"}
              ${!hasBackup ? "cursor-not-allowed" : "cursor-pointer"}
            `}
            aria-label="Toggle between main and backup view"
          >
            <span
              className={`
                absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white shadow
                transition-transform duration-200
                ${viewMode === "backup" ? "translate-x-4" : "translate-x-0"}
              `}
            />
          </button>
          <span
            className={`text-xs font-medium transition-colors ${
              viewMode === "backup" ? "text-white" : "text-gray-500"
            }`}
          >
            Backup
          </span>
        </div>

        {/* ── Spacer ──────────────────────────────────────────────────── */}
        <div className="flex-1" />

        {/* ── Theme toggle ────────────────────────────────────────────── */}
        <button
          onClick={toggleTheme}
          className="p-1.5 rounded-md text-gray-400 hover:text-white hover:bg-white/10 transition-colors"
          title={theme === "light" ? "Switch to dark mode" : "Switch to light mode"}
        >
          {theme === "light" ? (
            /* Moon icon */
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M21.752 15.002A9.718 9.718 0 0118 15.75c-5.385 0-9.75-4.365-9.75-9.75 0-1.33.266-2.597.748-3.752A9.753 9.753 0 003 11.25C3 16.635 7.365 21 12.75 21a9.753 9.753 0 009.002-5.998z" />
            </svg>
          ) : (
            /* Sun icon */
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M12 3v2.25m6.364.386l-1.591 1.591M21 12h-2.25m-.386 6.364l-1.591-1.591M12 18.75V21m-4.773-4.227l-1.591 1.591M5.25 12H3m4.227-4.773L5.636 5.636M15.75 12a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0z" />
            </svg>
          )}
        </button>

        {/* ── Action Buttons (page-specific) ──────────────────────────── */}
        {children && (
          <div className="flex items-center gap-2">{children}</div>
        )}

        {/* ── User dropdown ─────────────────────────────────────────── */}
        {username && (
          <div className="relative" ref={dropdownRef}>
            <button
              onClick={() => setDropdownOpen((v) => !v)}
              className="flex items-center gap-2 text-gray-400 hover:text-white text-xs border-l border-white/10 pl-4 ml-2 transition-colors"
            >
              <div className={`w-6 h-6 rounded-full bg-gradient-to-br from-blue-500 to-purple-600 flex items-center justify-center text-white text-xs font-bold uppercase${isProcessing ? " processing-ring" : ""}`}>
                {username.charAt(0)}
              </div>
              <span className="hidden sm:inline">{username}</span>
              <svg className={`w-3 h-3 transition-transform ${dropdownOpen ? "rotate-180" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M19.5 8.25l-7.5 7.5-7.5-7.5" />
              </svg>
            </button>

            {dropdownOpen && (
              <div className="absolute right-0 mt-2 w-44 bg-white dark:bg-gray-800 rounded-lg shadow-lg border border-gray-200 dark:border-gray-700 py-1 z-50">
                <button
                  onClick={() => { navigate("/secure-gallery"); setDropdownOpen(false); }}
                  className="w-full text-left px-4 py-2 text-sm text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 transition-colors"
                >
                  <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M16.5 10.5V6.75a4.5 4.5 0 10-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 002.25-2.25v-6.75a2.25 2.25 0 00-2.25-2.25H6.75a2.25 2.25 0 00-2.25 2.25v6.75a2.25 2.25 0 002.25 2.25z" />
                  </svg>
                  Secure Gallery
                </button>
                <button
                  onClick={() => { navigate("/settings"); setDropdownOpen(false); }}
                  className="w-full text-left px-4 py-2 text-sm text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 transition-colors"
                >
                  <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.324.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 011.37.49l1.296 2.247a1.125 1.125 0 01-.26 1.431l-1.003.827c-.293.24-.438.613-.431.992a6.759 6.759 0 010 .255c-.007.378.138.75.43.99l1.005.828c.424.35.534.954.26 1.43l-1.298 2.247a1.125 1.125 0 01-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.57 6.57 0 01-.22.128c-.331.183-.581.495-.644.869l-.213 1.28c-.09.543-.56.941-1.11.941h-2.594c-.55 0-1.02-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 01-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 01-1.369-.49l-1.297-2.247a1.125 1.125 0 01.26-1.431l1.004-.827c.292-.24.437-.613.43-.992a6.932 6.932 0 010-.255c.007-.378-.138-.75-.43-.99l-1.004-.828a1.125 1.125 0 01-.26-1.43l1.297-2.247a1.125 1.125 0 011.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.087.22-.128.332-.183.582-.495.644-.869l.214-1.281z" />
                    <path strokeLinecap="round" strokeLinejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                  </svg>
                  Settings
                </button>
                <div className="border-t border-gray-200 dark:border-gray-700 my-1" />
                <button
                  onClick={() => { handleLogout(); setDropdownOpen(false); }}
                  className="w-full text-left px-4 py-2 text-sm text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-900/30 flex items-center gap-2 transition-colors"
                >
                  <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 9V5.25A2.25 2.25 0 0013.5 3h-6a2.25 2.25 0 00-2.25 2.25v13.5A2.25 2.25 0 007.5 21h6a2.25 2.25 0 002.25-2.25V15m3 0l3-3m0 0l-3-3m3 3H9" />
                  </svg>
                  Sign Out
                </button>
              </div>
            )}
          </div>
        )}
      </div>
    </header>
  );
}
