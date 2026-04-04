/** Visual progress indicator showing completed/current/upcoming setup steps. */
import type { WizardStep, ServerRole, InstallType } from "./types";

export interface StepIndicatorProps {
  step: WizardStep;
  serverRole?: ServerRole;
  installType?: InstallType;
}

export default function StepIndicator({ step, serverRole, installType }: StepIndicatorProps) {
  // Different step lists depending on server role and install type
  const steps =
    serverRole === "backup"
      ? [
          { id: "welcome", label: "Welcome" },
          { id: "server-role", label: "Role" },
          { id: "pair", label: "Pair" },
          { id: "storage", label: "Server" },
          { id: "complete", label: "Done" },
        ]
      : installType === "restore"
        ? [
            { id: "welcome", label: "Welcome" },
            { id: "server-role", label: "Role" },
            { id: "install-type", label: "Type" },
            { id: "restore", label: "Restore" },
            { id: "storage", label: "Server" },
            { id: "ssl", label: "SSL" },
            { id: "android", label: "Android" },
            { id: "complete", label: "Done" },
          ]
        : [
            { id: "welcome", label: "Welcome" },
            { id: "server-role", label: "Role" },
            { id: "install-type", label: "Type" },
            { id: "account", label: "Account" },
            { id: "admin-2fa", label: "2FA" },
            { id: "storage", label: "Server" },
            { id: "ssl", label: "SSL" },
            { id: "users", label: "Users" },
            { id: "android", label: "Android" },
            { id: "complete", label: "Done" },
          ];
  // Map user-2fa to users for indicator purposes
  const displayStep = step === "user-2fa" ? "users" : step;
  const currentIdx = steps.findIndex((s) => s.id === displayStep);

  return (
    <div className="flex items-center justify-center gap-1 mb-8 flex-wrap">
      {steps.map((s, i) => (
        <div key={s.id} className="flex items-center gap-1">
          <div
            className={`w-7 h-7 rounded-full flex items-center justify-center text-xs font-medium transition-colors ${
              i < currentIdx
                ? "bg-green-500 text-white"
                : i === currentIdx
                  ? "bg-blue-600 text-white"
                  : "bg-gray-200 dark:bg-gray-600 text-gray-500 dark:text-gray-400"
            }`}
          >
            {i < currentIdx ? "\u2713" : i + 1}
          </div>
          <span
            className={`text-xs hidden sm:inline ${
              i === currentIdx
                ? "text-blue-600 font-medium"
                : "text-gray-400"
            }`}
          >
            {s.label}
          </span>
          {i < steps.length - 1 && (
            <div
              className={`w-4 h-0.5 ${
                i < currentIdx ? "bg-green-500" : "bg-gray-200 dark:bg-gray-600"
              }`}
            />
          )}
        </div>
      ))}
    </div>
  );
}
