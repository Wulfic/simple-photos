import { useEffect, useState } from "react";
import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
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

/**
 * Protected route guard.
 *
 * Checks setup status first:
 * - If no users exist yet, redirect to /welcome (first-time setup)
 * - If not authenticated, redirect to /login
 * - Otherwise, render children
 */
function ProtectedRoute({ children }: { children: React.ReactNode }) {
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
      .catch(() => {
        // Can't reach server — assume setup is complete, let auth handle it
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

  // No users exist — must complete first-time setup before anything else
  if (!setupComplete) return <Navigate to="/welcome" replace />;

  // Not logged in — must authenticate
  if (!isAuthenticated) return <Navigate to="/login" replace />;

  return <>{children}</>;
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
        <Route path="/welcome" element={<Welcome />} />
        <Route
          path="/login"
          element={
            <LoginGuard>
              <Login />
            </LoginGuard>
          }
        />
        <Route
          path="/setup"
          element={
            <ProtectedRoute>
              <Setup />
            </ProtectedRoute>
          }
        />
        <Route
          path="/gallery"
          element={
            <ProtectedRoute>
              <Gallery />
            </ProtectedRoute>
          }
        />
        <Route
          path="/albums"
          element={
            <ProtectedRoute>
              <Albums />
            </ProtectedRoute>
          }
        />
        <Route
          path="/albums/:albumId"
          element={
            <ProtectedRoute>
              <AlbumDetail />
            </ProtectedRoute>
          }
        />
        <Route
          path="/photo/plain/:id"
          element={
            <ProtectedRoute>
              <Viewer />
            </ProtectedRoute>
          }
        />
        <Route
          path="/photo/:id"
          element={
            <ProtectedRoute>
              <Viewer />
            </ProtectedRoute>
          }
        />
        <Route
          path="/settings"
          element={
            <ProtectedRoute>
              <Settings />
            </ProtectedRoute>
          }
        />
        <Route
          path="/trash"
          element={
            <ProtectedRoute>
              <Trash />
            </ProtectedRoute>
          }
        />
        <Route
          path="/shared/:albumId"
          element={
            <ProtectedRoute>
              <SharedAlbumDetail />
            </ProtectedRoute>
          }
        />
        <Route
          path="/search"
          element={
            <ProtectedRoute>
              <Search />
            </ProtectedRoute>
          }
        />
        <Route
          path="/secure-gallery"
          element={
            <ProtectedRoute>
              <SecureGallery />
            </ProtectedRoute>
          }
        />
        <Route
          path="/diagnostics"
          element={
            <ProtectedRoute>
              <Diagnostics />
            </ProtectedRoute>
          }
        />
        <Route path="*" element={<RootRedirect />} />
      </Routes>
    </BrowserRouter>
  );
}
