/**
 * usePwaInstall — captures the browser's `beforeinstallprompt` event so
 * the UI can offer a one-click "Install App" button.
 *
 * Behaviour:
 *  - `canInstall`     — true once the browser has fired the prompt event
 *                       (Chromium-based browsers only).
 *  - `isInstalled`    — true if the app is already running in standalone
 *                       mode (display-mode: standalone) or the
 *                       `appinstalled` event has fired.
 *  - `promptInstall()`— shows the native install dialog. Returns the
 *                       user's choice ("accepted" | "dismissed" | "unavailable").
 */
import { useCallback, useEffect, useState } from "react";

// Shape of the (non-standard but widely supported) BeforeInstallPromptEvent.
interface BeforeInstallPromptEvent extends Event {
  readonly platforms: string[];
  prompt: () => Promise<void>;
  readonly userChoice: Promise<{ outcome: "accepted" | "dismissed"; platform: string }>;
}

export function usePwaInstall() {
  const [deferredPrompt, setDeferredPrompt] = useState<BeforeInstallPromptEvent | null>(null);
  const [isInstalled, setIsInstalled] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    // iOS Safari exposes a non-standard `navigator.standalone`.
    const iosStandalone = (window.navigator as unknown as { standalone?: boolean }).standalone === true;
    return iosStandalone || window.matchMedia("(display-mode: standalone)").matches;
  });

  useEffect(() => {
    const onBeforeInstall = (e: Event) => {
      // Stop Chrome's automatic mini-infobar so we can present our own UI.
      e.preventDefault();
      setDeferredPrompt(e as BeforeInstallPromptEvent);
    };
    const onInstalled = () => {
      setIsInstalled(true);
      setDeferredPrompt(null);
    };
    window.addEventListener("beforeinstallprompt", onBeforeInstall);
    window.addEventListener("appinstalled", onInstalled);
    return () => {
      window.removeEventListener("beforeinstallprompt", onBeforeInstall);
      window.removeEventListener("appinstalled", onInstalled);
    };
  }, []);

  const promptInstall = useCallback(async (): Promise<"accepted" | "dismissed" | "unavailable"> => {
    if (!deferredPrompt) return "unavailable";
    await deferredPrompt.prompt();
    const choice = await deferredPrompt.userChoice;
    setDeferredPrompt(null);
    return choice.outcome;
  }, [deferredPrompt]);

  return {
    canInstall: deferredPrompt !== null,
    isInstalled,
    promptInstall,
  };
}
