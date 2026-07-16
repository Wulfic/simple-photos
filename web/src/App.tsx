import { useEffect, useState, lazy, Suspense } from "react";
import { BrowserRouter, Routes, Route, Navigate, Outlet } from "react-router-dom";
import { useAuthStore } from "./store/auth";
import { useThemeStore } from "./store/theme";
import { hasCryptoKey } from "./crypto/crypto";
import { startKeySessionResponder } from "./crypto/keySession";
import { restoreKeyOnBoot } from "./crypto/restoreKey";
import RouteFallback from "./components/RouteFallback";

// Route pages are code-split with React.lazy so the initial bundle only ships
// the shell + the first route's chunk. Each lazy route is caught by a
// <Suspense> boundary (RouteFallback for protected pages; a minimal boot
// fallback for the public/guard routes) so the chunk download shows an
// intentional loading state rather than a blank screen.
const Login = lazy(() => import("./pages/Login"));
const Setup = lazy(() => import("./pages/Setup"));
const Gallery = lazy(() => import("./pages/Gallery"));
const Albums = lazy(() => import("./pages/Albums"));
const AlbumDetail = lazy(() => import("./pages/AlbumDetail"));
const Viewer = lazy(() => import("./pages/Viewer"));
const Settings = lazy(() => import("./pages/Settings"));
const Welcome = lazy(() => import("./pages/Welcome"));
const Trash = lazy(() => import("./pages/Trash"));
const SecureGallery = lazy(() => import("./pages/SecureGallery"));
const SharedAlbumDetail = lazy(() => import("./pages/SharedAlbumDetail"));
const Search = lazy(() => import("./pages/Search"));
const Diagnostics = lazy(() => import("./pages/Diagnostics"));
const ExportDownloads = lazy(() => import("./pages/ExportDownloads"));
const CastReceiver = lazy(() => import("./pages/CastReceiver"));
import useSyncEvents from "./hooks/useSyncEvents";
import BannerHost from "./components/BannerHost";
import EncryptionBanner from "./components/EncryptionBanner";
import ConversionBanner from "./components/ConversionBanner";
import SavingBanner from "./components/SavingBanner";
import AiBanner from "./components/AiBanner";
import GeoBanner from "./components/GeoBanner";
import PreciseGeoBanner from "./components/PreciseGeoBanner";
import ServerOfflineBanner from "./components/ServerOfflineBanner";
import ToastHost from "./components/ToastHost";

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
  const [wizardCompleted, setWizardCompleted] = useState(true);
  const [serverUnreachable, setServerUnreachable] = useState(false);

  // Subscribe to the server's real-time album/gallery change stream so this
  // client refetches within seconds of a change made elsewhere (item #11).
  useSyncEvents();

  useEffect(() => {
    fetch("/api/setup/status")
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.json();
      })
      .then((data) => {
        // Treat the *wizard* as the gate, not just "a user exists". The
        // wizard may have created an admin but not yet been finalized
        // (e.g. browser crashed mid-flow); we still need to send the
        // user back to /welcome rather than into the gallery.
        setWizardCompleted(Boolean(data.wizard_completed));
        setSetupChecked(true);
      })
      .catch(() => {
        setServerUnreachable(true);
        setSetupChecked(true);
      });
  }, []);

  if (!setupChecked) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-canvas">
        <div className="w-8 h-8 border-4 border-accent-600 border-t-transparent rounded-full animate-spin" />
      </div>
    );
  }

  if (serverUnreachable) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-canvas p-4">
        {/* Runtime offline banner stays visible even on this error page */}
        <ServerOfflineBanner />
        <div className="text-center max-w-md">
          <h1 className="text-xl font-semibold text-fg mb-2">Cannot reach server</h1>
          <p className="text-fg-muted mb-4">
            Unable to connect to the Simple Photos server. Check that the server is running, then retry.
          </p>
          <button
            onClick={() => window.location.reload()}
            className="btn btn-primary btn-md"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  // Wizard not finalized — send the user back to step 1 of the wizard.
  // (The Welcome page itself decides whether to resume from sessionStorage
  // or restart from the welcome step.)
  if (!wizardCompleted) return <Navigate to="/welcome" replace />;

  // Not logged in — must authenticate
  if (!isAuthenticated) return <Navigate to="/login" replace />;

  return (
    <>
      <ToastHost />
      {/* Single fixed container all progress banners portal into — see item #3.
          Must render before the banners so the portal target exists. */}
      <BannerHost />
      <ConversionBanner />
      <EncryptionBanner />
      <AiBanner />
      <GeoBanner />
      <PreciseGeoBanner />
      <SavingBanner />
      {/* Route-content wrapper carries `view-transition-name: page` so only
          the page body crossfades on navigation (see index.css). The fixed
          AppHeader rendered inside each page has its own `app-header` name and
          is lifted out of this snapshot, so it never flickers. */}
      <div className="[view-transition-name:page]">
        <Suspense fallback={<RouteFallback />}>
          <Outlet />
        </Suspense>
      </div>
    </>
  );
}

/**
 * Login page guard — if setup is not complete, redirect to /welcome instead.
 */
function LoginGuard({ children }: { children: React.ReactNode }) {
  const { isAuthenticated } = useAuthStore();
  const [setupChecked, setSetupChecked] = useState(false);
  const [wizardCompleted, setWizardCompleted] = useState(true);

  useEffect(() => {
    fetch("/api/setup/status")
      .then((r) => r.json())
      .then((data) => {
        setWizardCompleted(Boolean(data.wizard_completed));
        setSetupChecked(true);
      })
      .catch(() => setSetupChecked(true));
  }, []);

  if (!setupChecked) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-canvas">
        <div className="w-8 h-8 border-4 border-accent-600 border-t-transparent rounded-full animate-spin" />
      </div>
    );
  }

  if (!wizardCompleted) return <Navigate to="/welcome" replace />;
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
        if (!data.wizard_completed) {
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
      <div className="min-h-screen flex items-center justify-center bg-canvas">
        <div className="w-8 h-8 border-4 border-accent-600 border-t-transparent rounded-full animate-spin" />
      </div>
    );
  }

  return <Navigate to={target} replace />;
}

/** How long boot waits for the encryption key to be restored from the keystore
 *  before giving up and rendering anyway (see the restore in [App]). */
const KEY_RESTORE_TIMEOUT_MS = 3000;

/** Minimal full-screen fallback for the outer Suspense boundary (public/guard
 * route chunks). Mirrors the existing setup-check spinner. */
function BootFallback() {
  return (
    <div className="min-h-screen flex items-center justify-center bg-canvas">
      <div className="w-8 h-8 border-4 border-accent-600 border-t-transparent rounded-full animate-spin" />
    </div>
  );
}

export default function App() {
  const { loadFromStorage } = useAuthStore();
  const { theme } = useThemeStore();
  const [keyReady, setKeyReady] = useState(false);

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

    // Vouch for this tab's session while it holds the key, so a tab opened
    // later can adopt it rather than re-deriving from the password (#21).
    const stopResponder = startKeySessionResponder(hasCryptoKey);

    // A reload — or a window.open'd tab, which inherits a copy of
    // sessionStorage — still has the key flag and loads the key directly. A
    // hand-opened tab has no flag: the key is in IndexedDB, but adopting it
    // needs a live peer to confirm the session is still running.
    const restore = restoreKeyOnBoot({
      isAuthenticated: useAuthStore.getState().isAuthenticated,
    });

    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;

    // The restore reads IndexedDB, and it now gates the first render — so a
    // keystore that never answers (corrupt IDB, a blocked upgrade) must not
    // mean a permanent spinner. On timeout we render anyway and degrade to the
    // previous behavior: the sessionStorage flag decides what the pages do, and
    // encrypt/decrypt re-await the load on demand.
    const bootGuard = new Promise<void>((resolve) => {
      timer = setTimeout(() => {
        console.error("Encryption-key restore timed out on boot — rendering without it");
        resolve();
      }, KEY_RESTORE_TIMEOUT_MS);
    });

    Promise.race([restore, bootGuard]).finally(() => {
      clearTimeout(timer);
      if (!cancelled) setKeyReady(true);
    });

    return () => {
      cancelled = true;
      clearTimeout(timer);
      stopResponder();
    };
  }, []);

  // Pages decide whether to bounce to /setup by reading the key synchronously
  // (hasCryptoKey), so nothing may render until the restore above settles —
  // otherwise a tab adopting a peer's key would race its own gallery to the
  // password screen.
  if (!keyReady) return <BootFallback />;

  return (
    <BrowserRouter>
      {/* Global server-health banner — shows on every page when the server
          is unreachable at runtime and auto-hides once it reconnects. */}
      <ServerOfflineBanner />
      {/* Outer boundary: catches the lazy chunk load for public/guard routes
          (login, welcome, cast) where a full-page grid skeleton wouldn't fit —
          a neutral centered spinner matches those screens. Protected routes
          have their own closer <Suspense> with RouteFallback. */}
      <Suspense fallback={<BootFallback />}>
      <Routes>
        {/* Public routes — no auth required */}
        {/* Cast receiver — must be public so Chromecast can load it */}
        <Route path="/cast-view" element={<CastReceiver />} />
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
          <Route path="/albums/:albumId/:subId" element={<AlbumDetail />} />
          <Route path="/photo/:id" element={<Viewer />} />
          <Route path="/settings" element={<Settings />} />
          <Route path="/trash" element={<Trash />} />
          <Route path="/shared/:albumId" element={<SharedAlbumDetail />} />
          <Route path="/search" element={<Search />} />
          <Route path="/secure-gallery" element={<SecureGallery />} />
          <Route path="/diagnostics" element={<Diagnostics />} />
          <Route path="/export-downloads" element={<ExportDownloads />} />
        </Route>

        <Route path="*" element={<RootRedirect />} />
      </Routes>
      </Suspense>
    </BrowserRouter>
  );
}
