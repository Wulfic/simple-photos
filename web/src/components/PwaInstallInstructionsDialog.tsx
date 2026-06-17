/**
 * PwaInstallInstructionsDialog — fallback dialog shown when the browser
 * has not (yet) fired `beforeinstallprompt` and we therefore cannot trigger
 * a native install programmatically.
 *
 * Browsers like Brave (with shields on), Firefox, and Safari either delay or
 * disable the install-prompt event entirely, so the user still needs an
 * actionable path to install the PWA. This dialog detects the active
 * browser/platform and shows the matching manual install steps.
 */
import { useEffect } from "react";

export type InstallEnv =
  | "brave"
  | "chromium-desktop"
  | "chromium-mobile"
  | "firefox-desktop"
  | "firefox-mobile"
  | "safari-ios"
  | "safari-mac"
  | "edge-desktop"
  | "unknown";

/** Detect the current browser/platform for install-instruction purposes.
 *  Only used to pick which help text to show — never security-sensitive. */
export function detectInstallEnv(): InstallEnv {
  if (typeof navigator === "undefined") return "unknown";
  const ua = navigator.userAgent || "";
  const isMobile = /Android|iPhone|iPad|iPod|Mobile/i.test(ua);

  // Brave exposes `navigator.brave.isBrave()` (async on some versions);
  // also leaves "Brave" out of UA on desktop, so we feature-detect.
  const nav = navigator as unknown as { brave?: { isBrave?: () => unknown } };
  if (nav.brave && typeof nav.brave.isBrave === "function") return "brave";

  if (/Edg\//.test(ua)) return "edge-desktop";

  if (/Firefox\//.test(ua)) {
    return isMobile ? "firefox-mobile" : "firefox-desktop";
  }

  // iOS Safari and any browser on iOS (all use WebKit; install path is the same).
  if (/iPad|iPhone|iPod/i.test(ua)) return "safari-ios";

  // Desktop Safari
  if (/Safari\//.test(ua) && !/Chrome\//.test(ua) && !/Chromium\//.test(ua)) {
    return "safari-mac";
  }

  if (/Chrome\//.test(ua) || /Chromium\//.test(ua)) {
    return isMobile ? "chromium-mobile" : "chromium-desktop";
  }

  return "unknown";
}

interface InstructionStep {
  title: string;
  steps: string[];
  note?: string;
}

function instructionsFor(env: InstallEnv): InstructionStep {
  switch (env) {
    case "brave":
      return {
        title: "Install on Brave",
        steps: [
          'Click the Brave menu (☰) in the top-right corner.',
          'Choose "Install Simple Photos…" — or "More tools → Create shortcut…" on older Brave versions.',
          'Confirm in the dialog to add it as a desktop app.',
        ],
        note:
          "Brave Shields can suppress the automatic install prompt. If you don't see " +
          '"Install Simple Photos…" in the menu, lower Shields on this site (lion icon → Shields down) ' +
          "and reload — the option will then appear.",
      };
    case "chromium-desktop":
      return {
        title: "Install on Chrome / Chromium",
        steps: [
          'Look for the install icon (a monitor with a down-arrow) at the right edge of the address bar and click it.',
          'If you don\'t see the icon, open the ⋮ menu and choose "Install Simple Photos…" or "Cast, save and share → Install page as app…".',
          "Confirm in the dialog.",
        ],
      };
    case "edge-desktop":
      return {
        title: "Install on Microsoft Edge",
        steps: [
          'Click the ⋯ menu in the top-right.',
          'Choose "Apps → Install this site as an app".',
          "Confirm in the dialog.",
        ],
      };
    case "chromium-mobile":
      return {
        title: "Install on Android Chrome",
        steps: [
          'Tap the ⋮ menu (top-right).',
          'Choose "Install app" or "Add to Home screen".',
          "Confirm to add the icon to your home screen.",
        ],
      };
    case "firefox-desktop":
      return {
        title: "Install on Firefox (desktop)",
        steps: [
          "Firefox desktop does not currently support installing web apps natively.",
          "You can bookmark this site or use a Chromium-based browser (Chrome, Edge, Brave) to install it as an app.",
        ],
      };
    case "firefox-mobile":
      return {
        title: "Install on Firefox (Android)",
        steps: [
          'Tap the ⋮ menu.',
          'Choose "Install" or "Add to Home screen".',
          "Confirm to place the icon on your home screen.",
        ],
      };
    case "safari-ios":
      return {
        title: "Install on iPhone / iPad",
        steps: [
          "Tap the Share button (the square with an up-arrow) at the bottom (iPhone) or top (iPad) of Safari.",
          'Scroll the share sheet and tap "Add to Home Screen".',
          'Tap "Add" in the top-right to confirm.',
        ],
        note:
          "This must be done from Safari — Chrome and other browsers on iOS cannot install web apps " +
          "because Apple restricts that capability to Safari/WebKit.",
      };
    case "safari-mac":
      return {
        title: "Install on Safari (macOS)",
        steps: [
          'In the Safari menu bar choose File → "Add to Dock…".',
          'Confirm the name and click "Add".',
        ],
        note: "Requires macOS Sonoma (14) or newer.",
      };
    case "unknown":
    default:
      return {
        title: "Install Simple Photos",
        steps: [
          "Your browser hasn't offered an automatic install prompt.",
          "Open your browser menu and look for an option named one of: " +
            '"Install app", "Install this site as an app", "Add to Home Screen", or "Create shortcut".',
          "Confirm in the dialog to finish installing.",
        ],
      };
  }
}

export default function PwaInstallInstructionsDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  // Close on Escape for keyboard users.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const env = detectInstallEnv();
  const { title, steps, note } = instructionsFor(env);

  return (
    <div
      className="fixed inset-0 bg-black/50 z-50 flex items-center justify-center p-4"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-labelledby="pwa-install-title"
    >
      <div
        className="bg-white dark:bg-gray-800 rounded-lg shadow-xl max-w-md w-full p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-start justify-between mb-3">
          <h3
            id="pwa-install-title"
            className="text-base font-semibold text-gray-900 dark:text-white"
          >
            {title}
          </h3>
          <button
            onClick={onClose}
            className="text-gray-600 dark:text-gray-400 hover:text-gray-600 dark:hover:text-gray-200"
            aria-label="Close"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <ol className="list-decimal pl-5 space-y-1.5 text-sm text-gray-700 dark:text-gray-300">
          {steps.map((s, i) => (
            <li key={i}>{s}</li>
          ))}
        </ol>

        {note && (
          <p className="mt-3 text-xs text-gray-700 dark:text-gray-400 italic">{note}</p>
        )}

        <div className="mt-5 flex justify-end">
          <button
            onClick={onClose}
            className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm font-medium"
          >
            Got it
          </button>
        </div>
      </div>
    </div>
  );
}
