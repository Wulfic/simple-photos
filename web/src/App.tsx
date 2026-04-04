import { useEffect, useState } from "react";
import { BrowserRouter, Routes, Route, Navigate, Outlet } from "react-router-dom";
import { useAuthStore } from "./store/auth";
import { useThemeStore } from "./store/theme";
import { hasCryptoKey, loadKeyFromSession } from "./crypto/crypto";
import Login from "./pages/Login";
import Setup from "./pages/Setup";
import Gallery from "./pages/Gallery";
import Albums from "./pages/Albums";
import AlbumDetail from "./pages/AlbumDetail";
import Viewer from "./pages/Viewer";
import Settings from "./pages/Settings";
import Welcome from "./pages/Welcome";
import Trash from "./pages/Trash";
import SecureGallery from "./pages/SecureGallery";
import SharedAlbumDetail from "./pages/SharedAlbumDetail";
import Search from "./pages/Search";
import Diagnostics from "./pages/Diagnostics";
import EncryptionBanner from "./components/EncryptionBanner";

/**
 * Layout route for authenticated pages.
 *
 * Checks setup status + auth and renders child pages via `<Outlet />`.
 *
 * Because this is a layout route, it does NOT remount when navigating between
 * child routes.
 */
function ProtectedLayout() {
  const { isAuthenticated } = useAuthStore();
  const [setupChecked, setSetupChecked] = useState(false);
  const [setupComplete, setSetupComplete] = useState(true);
  const [serverUnreachable, setServerUnreachable] = useState(false);

  useEffect(() => {
    fetch("/api/setup/status")
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.json();
      })
      .then((data) => {
        setSetupComplete(data.setup_complete);
        setSetupChecked(true);
      })
      .catch(() => {
        setServerUnreachable(true);
        setSetupChecked(true);
      });
  }, []);

  if (!setupChecked) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900">
        <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
      </div>
    );
  }

  if (serverUnreachable) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900 p-4">
        <div className="text-center max-w-md">
          <h1 className="text-xl font-semibold text-gray-900 dark:text-gray-100 mb-2">Cannot reach server</h1>
          <p className="text-gray-600 dark:text-gray-400 mb-4">
            Unable to connect to the Simple Photos server. Please check that the server is running and try again.
          </p>
          <button
            onClick={() => window.location.reload()}
            className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  // No users exist — must complete first-time setup before anything else
  if (!setupComplete) return <Navigate to="/welcome" replace />;

  // Not logged in — must authenticate
  if (!isAuthenticated) return <Navigate to="/login" replace />;

  return (
    <>
      <EncryptionBanner />
      <Outlet />
    </>
  );
}

/**
 * Login page guard — if setup is not complete, redirect to /welcome instead.
 */
function LoginGuard({ children }: { children: React.ReactNode }) {
  const { isAuthenticated } = useAuthStore();
  const [setupChecked, setSetupChecked] = useState(false);
  const [setupComplete, setSetupComplete] = useState(true);

  useEffect(() => {
    fetch("/api/setup/status")
      .then((r) => r.json())
      .then((data) => {
        setSetupComplete(data.setup_complete);
        setSetupChecked(true);
      })
      .catch(() => setSetupChecked(true));
  }, []);

  if (!setupChecked) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900">
        <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
      </div>
    );
  }

  if (!setupComplete) return <Navigate to="/welcome" replace />;
  if (isAuthenticated) return <Navigate to="/gallery" replace />;

  return <>{children}</>;
}

/**
 * Smart root redirect:
 * - If setup not complete → /welcome
 * - If authenticated → /gallery
 * - Otherwise → /login
 */
function RootRedirect() {
  const { isAuthenticated } = useAuthStore();
  const [target, setTarget] = useState<string | null>(null);

  useEffect(() => {
    fetch("/api/setup/status")
      .then((r) => r.json())
      .then((data) => {
        if (!data.setup_complete) {
          setTarget("/welcome");
        } else if (isAuthenticated) {
          setTarget("/gallery");
        } else {
          setTarget("/login");
        }
      })
      .catch(() => {
        // Can't reach server — fall back to login
        setTarget(isAuthenticated ? "/gallery" : "/login");
      });
  }, [isAuthenticated]);

  if (!target) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900">
        <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
      </div>
    );
  }

  return <Navigate to={target} replace />;
}

export default function App() {
  const { loadFromStorage } = useAuthStore();
  const { theme } = useThemeStore();

  // Apply dark class to <html> element
  useEffect(() => {
    const root = document.documentElement;
    if (theme === "dark") {
      root.classList.add("dark");
    } else {
      root.classList.remove("dark");
    }
  }, [theme]);

  useEffect(() => {
    loadFromStorage();
    loadKeyFromSession();
  }, []);

  return (
    <BrowserRouter>
      <Routes>
        {/* Public routes — no auth required */}
        <Route path="/welcome" element={<Welcome />} />
        <Route
          path="/login"
          element={
            <LoginGuard>
              <Login />
            </LoginGuard>
          }
        />

        {/*
         * Protected layout route — auth check + activity poller + banners.
         * This element persists across child route navigation, so the poller
         * never stops and the banners never unmount during page changes.
         */}
        <Route element={<ProtectedLayout />}>
          <Route path="/setup" element={<Setup />} />
          <Route path="/gallery" element={<Gallery />} />
          <Route path="/albums" element={<Albums />} />
          <Route path="/albums/:albumId" element={<AlbumDetail />} />
          <Route path="/photo/:id" element={<Viewer />} />
          <Route path="/settings" element={<Settings />} />
          <Route path="/trash" element={<Trash />} />
          <Route path="/shared/:albumId" element={<SharedAlbumDetail />} />
          <Route path="/search" element={<Search />} />
          <Route path="/secure-gallery" element={<SecureGallery />} />
          <Route path="/diagnostics" element={<Diagnostics />} />
        </Route>

        <Route path="*" element={<RootRedirect />} />
      </Routes>
    </BrowserRouter>
  );
}
