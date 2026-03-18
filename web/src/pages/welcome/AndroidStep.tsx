/** Wizard step — Android app download link and server URL QR code. */
import { useState } from "react";
import type { WizardStep } from "./types";
import AppIcon from "../../components/AppIcon";

export interface AndroidStepProps {
  setStep: (step: WizardStep) => void;
  setError: (msg: string) => void;
}

export default function AndroidStep({ setStep, setError }: AndroidStepProps) {
  const [downloading, setDownloading] = useState(false);

  /** Programmatic download — avoids the double-download issue that <a> tags
   *  can cause in SPAs (browser navigates + downloads simultaneously). */
  async function handleDownload() {
    if (downloading) return; // guard against double-click
    setDownloading(true);
    setError("");
    try {
      // HEAD check first to give a helpful error if the APK hasn't been built
      const check = await fetch("/api/downloads/android", { method: "HEAD" });
      if (!check.ok) {
        setError(
          "Android APK is not available. Build it with: cd android && ./gradlew assembleRelease — or place a pre-built APK at downloads/simple-photos.apk"
        );
        return;
      }
      // Trigger the actual download via a temporary anchor with `download`
      // attribute, preventing the browser from navigating the SPA away.
      const a = document.createElement("a");
      a.href = "/api/downloads/android";
      a.download = "simple-photos.apk";
      document.body.appendChild(a);
      a.click();
      a.remove();
    } catch {
      setError("Could not download the APK. Is the server running?");
    } finally {
      // Brief cooldown to prevent accidental double-clicks
      setTimeout(() => setDownloading(false), 2000);
    }
  }

  return (
    <div>
      <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-1">
        Set Up Android App
      </h2>
      <p className="text-gray-500 dark:text-gray-400 text-sm mb-6">
        Install the Simple Photos app on your Android device for
        automatic photo backup.
      </p>

      <div className="space-y-4">
        {/* Download button — uses programmatic download to avoid SPA navigation issues */}
        <button
          onClick={handleDownload}
          disabled={downloading}
          className="flex items-center justify-center gap-3 w-full bg-green-600 text-white py-3 rounded-lg hover:bg-green-700 disabled:opacity-50 text-sm font-medium transition-colors"
        >
          <AppIcon name="download" size="w-6 h-6" />
          {downloading ? "Downloading…" : "Download APK"}
        </button>

        {/* Sideloading instructions */}
        <div className="bg-gray-50 dark:bg-gray-900 rounded-lg p-4">
          <h3 className="font-medium text-gray-800 dark:text-gray-200 text-sm mb-3">
            How to install (sideload):
          </h3>
          <ol className="text-sm text-gray-600 dark:text-gray-400 space-y-3">
            <li className="flex gap-3">
              <span className="flex-shrink-0 w-6 h-6 bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 rounded-full flex items-center justify-center text-xs font-bold">
                1
              </span>
              <div>
                <p className="font-medium text-gray-700 dark:text-gray-300">
                  Download the APK
                </p>
                <p className="text-xs text-gray-500 dark:text-gray-400">
                  Click the button above or transfer the APK to your
                  phone via USB/email.
                </p>
              </div>
            </li>
            <li className="flex gap-3">
              <span className="flex-shrink-0 w-6 h-6 bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 rounded-full flex items-center justify-center text-xs font-bold">
                2
              </span>
              <div>
                <p className="font-medium text-gray-700 dark:text-gray-300">
                  Enable "Install unknown apps"
                </p>
                <p className="text-xs text-gray-500 dark:text-gray-400">
                  Go to{" "}
                  <strong>
                    Settings → Apps → Special access → Install unknown
                    apps
                  </strong>{" "}
                  and enable it for your file manager or browser.
                </p>
              </div>
            </li>
            <li className="flex gap-3">
              <span className="flex-shrink-0 w-6 h-6 bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 rounded-full flex items-center justify-center text-xs font-bold">
                3
              </span>
              <div>
                <p className="font-medium text-gray-700 dark:text-gray-300">
                  Open the APK
                </p>
                <p className="text-xs text-gray-500 dark:text-gray-400">
                  Tap the downloaded APK file and confirm the
                  installation prompt.
                </p>
              </div>
            </li>
            <li className="flex gap-3">
              <span className="flex-shrink-0 w-6 h-6 bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 rounded-full flex items-center justify-center text-xs font-bold">
                4
              </span>
              <div>
                <p className="font-medium text-gray-700 dark:text-gray-300">
                  Connect to your server
                </p>
                <p className="text-xs text-gray-500 dark:text-gray-400">
                  Open the app, enter your server URL:
                </p>
                <code className="block mt-1 bg-gray-200 dark:bg-gray-600 px-2 py-1 rounded text-xs text-gray-800 dark:text-gray-200 break-all">
                  {window.location.origin}
                </code>
              </div>
            </li>
            <li className="flex gap-3">
              <span className="flex-shrink-0 w-6 h-6 bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 rounded-full flex items-center justify-center text-xs font-bold">
                5
              </span>
              <div>
                <p className="font-medium text-gray-700 dark:text-gray-300">
                  Sign in & grant permissions
                </p>
                <p className="text-xs text-gray-500 dark:text-gray-400">
                  Log in with your account and allow the app to access
                  your photos and videos for automatic encrypted backup.
                </p>
              </div>
            </li>
          </ol>
        </div>

        <div className="bg-amber-50 dark:bg-amber-900/30 border border-amber-200 dark:border-amber-800 rounded-lg p-3 text-xs text-amber-800 dark:text-amber-300">
          <strong>Note:</strong> Keep "Install unknown apps" disabled
          after installation for security. You can always re-enable it
          when updating the app.
        </div>
      </div>

      <div className="flex gap-3 mt-6">
        <button
          onClick={() => {
            setError("");
            setStep("users");
          }}
          className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 text-sm font-medium transition-colors"
        >
          ← Back
        </button>
        <button
          onClick={() => {
            setError("");
            setStep("complete");
          }}
          className="flex-[2] bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 text-sm font-medium transition-colors"
        >
          Continue →
        </button>
      </div>
    </div>
  );
}
