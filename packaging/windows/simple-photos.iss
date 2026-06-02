; ============================================================================
;  Simple Photos — Windows Installer (Inno Setup 6.x)
; ============================================================================
;
;  Builds a self-contained .exe installer that:
;    1. Asks the user for an install location (default: %ProgramFiles%\SimplePhotos).
;    2. Copies the pre-built server.exe + web\ + migrations\ into that location.
;    3. Generates %ProgramData%\SimplePhotos\config.toml with a random JWT secret.
;    4. Registers a Windows Service (auto-start, recovery, runs as LocalSystem).
;    5. Adds a Start Menu shortcut and a firewall rule for port 8080 (Private+Domain).
;    6. Schedules the post-install asset fetcher (ONNX + GeoNames) as a one-shot task.
;
;  Build with:  iscc packaging\windows\simple-photos.iss
;  CI invokes this from .github\workflows\release.yml (windows-latest runner).
;
;  Pre-requisites for the build (CI installs these automatically):
;    - The compiled binary at:  ..\..\server\target\release\simple-photos-server.exe
;    - The web build output:    ..\..\web\dist\
;    - The migrations folder:   ..\..\server\migrations\
;    - NSSM (https://nssm.cc) bundled at:  vendor\nssm.exe   (32 KB)
; ============================================================================

#define AppName        "Simple Photos"
#define AppPublisher   "Wulfic"
#define AppURL         "https://github.com/Wulfic/simple-photos"
#define AppExeName     "simple-photos-server.exe"
#define AppId          "{{B7A3F8C2-9D1E-4F2B-8E5A-1C2D3E4F5A6B}"
; SP_VERSION is supplied by CI via /DSP_VERSION=x.y.z. Falls back to 1.0.0.
#ifndef SP_VERSION
  #define SP_VERSION "1.1.35"
#endif

[Setup]
AppId={#AppId}
AppName={#AppName}
AppVersion={#SP_VERSION}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases
DefaultDirName={autopf}\SimplePhotos
DefaultGroupName=Simple Photos
DisableProgramGroupPage=no
DisableDirPage=no
LicenseFile=..\..\LICENSE
OutputDir=..\..\dist
OutputBaseFilename=simple-photos-{#SP_VERSION}-windows-x64-setup
Compression=lzma2/ultra
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=dialog
WizardStyle=modern
UninstallDisplayIcon={app}\{#AppExeName}
SetupLogging=yes
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
; Checked by default: Simple Photos is a server and most users expect it to
; run on boot. Unchecking installs files only (run manually / foreground).
Name: "service";    Description: "Install Simple Photos as a Windows Service (auto-start on boot)"; GroupDescription: "Service:"
Name: "firewall";   Description: "Add Windows Firewall rule for TCP port 8080 (Private + Domain networks)"; GroupDescription: "Network:"
; Unchecked by default: only useful with an NVIDIA GPU and pulls ~0.6-1 GB of
; CUDA 12 + cuDNN 9 runtime DLLs. Without it, AI inference runs on CPU.
Name: "gpu";        Description: "Enable NVIDIA GPU acceleration for AI (downloads CUDA 12 + cuDNN 9 runtime, ~0.6-1 GB)"; GroupDescription: "AI acceleration:"; Flags: unchecked

[Files]
; ── Server binary ──────────────────────────────────────────────────────────
Source: "..\..\server\target\release\simple-photos-server.exe"; DestDir: "{app}"; Flags: ignoreversion

; ── Web frontend ───────────────────────────────────────────────────────────
Source: "..\..\web\dist\*"; DestDir: "{app}\web"; Flags: ignoreversion recursesubdirs createallsubdirs

; ── DB migrations ──────────────────────────────────────────────────────────
Source: "..\..\server\migrations\*"; DestDir: "{app}\migrations"; Flags: ignoreversion recursesubdirs createallsubdirs

; ── NSSM (service wrapper) ─────────────────────────────────────────────────
; A tiny, BSD-licensed service shim — Simple Photos is a console app and
; Windows Services need a hosting wrapper. NSSM also gives us automatic
; restart on crash and clean log rotation.
Source: "vendor\nssm.exe"; DestDir: "{app}\bin"; Flags: ignoreversion

; ── Post-install asset fetcher ─────────────────────────────────────────────
Source: "fetch-assets.ps1"; DestDir: "{app}\bin"; Flags: ignoreversion

; ── CUDA runtime fetcher (optional GPU component) ──────────────────────────
Source: "fetch-cuda-runtime.ps1"; DestDir: "{app}\bin"; Flags: ignoreversion

; ── Documentation ──────────────────────────────────────────────────────────
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Dirs]
; Data root lives in ProgramData (per-machine, survives upgrades).
Name: "{commonappdata}\SimplePhotos";          Permissions: users-modify
Name: "{commonappdata}\SimplePhotos\db";       Permissions: users-modify
Name: "{commonappdata}\SimplePhotos\storage";  Permissions: users-modify
Name: "{commonappdata}\SimplePhotos\models";   Permissions: users-modify
Name: "{commonappdata}\SimplePhotos\logs";     Permissions: users-modify

[Icons]
Name: "{group}\Open Simple Photos in browser"; Filename: "http://localhost:8080"
Name: "{group}\Start service";                 Filename: "{sys}\sc.exe"; Parameters: "start SimplePhotos"; Tasks: service
Name: "{group}\Stop service";                  Filename: "{sys}\sc.exe"; Parameters: "stop SimplePhotos"; Tasks: service
Name: "{group}\Run server (foreground)";       Filename: "{app}\{#AppExeName}"; WorkingDir: "{app}"
Name: "{group}\Open data folder";              Filename: "{commonappdata}\SimplePhotos"
Name: "{group}\Uninstall Simple Photos";       Filename: "{uninstallexe}"

[Run]
; ── Generate config.toml on first install ─────────────────────────────────
Filename: "powershell.exe"; \
    Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\bin\fetch-assets.ps1"" -GenerateConfig -InstallDir ""{app}"" -DataDir ""{commonappdata}\SimplePhotos"""; \
    StatusMsg: "Generating configuration..."; \
    Flags: runhidden

; ── Install + start service (optional task) ───────────────────────────────
Filename: "{app}\bin\nssm.exe"; \
    Parameters: "install SimplePhotos ""{app}\{#AppExeName}"""; \
    Flags: runhidden; \
    Tasks: service
Filename: "{app}\bin\nssm.exe"; \
    Parameters: "set SimplePhotos AppDirectory ""{app}"""; \
    Flags: runhidden; \
    Tasks: service
Filename: "{app}\bin\nssm.exe"; \
    Parameters: "set SimplePhotos AppEnvironmentExtra SIMPLE_PHOTOS_CONFIG=""{commonappdata}\SimplePhotos\config.toml"" RUST_LOG=info PATH=""{app}\bin;%PATH%"""; \
    Flags: runhidden; \
    Tasks: service
Filename: "{app}\bin\nssm.exe"; \
    Parameters: "set SimplePhotos AppStdout ""{commonappdata}\SimplePhotos\logs\server.log"""; \
    Flags: runhidden; \
    Tasks: service
Filename: "{app}\bin\nssm.exe"; \
    Parameters: "set SimplePhotos AppStderr ""{commonappdata}\SimplePhotos\logs\server.log"""; \
    Flags: runhidden; \
    Tasks: service
Filename: "{app}\bin\nssm.exe"; \
    Parameters: "set SimplePhotos Start SERVICE_AUTO_START"; \
    Flags: runhidden; \
    Tasks: service
; ── Restart behaviour ─────────────────────────────────────────────────────
; Wait 15 seconds before restarting the server after it exits (crash or
; manual kill). This gives the user enough time to also stop the Windows
; Service (via "Stop service" shortcut, services.msc, or Task Manager →
; Services tab) before NSSM restarts the process. Without this delay NSSM
; would respawn immediately, making a Task Manager kill ineffective.
Filename: "{app}\bin\nssm.exe"; \
    Parameters: "set SimplePhotos AppRestartDelay 15000"; \
    Flags: runhidden; \
    Tasks: service
Filename: "{sys}\sc.exe"; \
    Parameters: "start SimplePhotos"; \
    StatusMsg: "Starting Simple Photos service..."; \
    Flags: runhidden; \
    Tasks: service

; ── Firewall rule ─────────────────────────────────────────────────────────
Filename: "powershell.exe"; \
    Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""New-NetFirewallRule -DisplayName 'SimplePhotos-Port-8080' -Direction Inbound -Protocol TCP -LocalPort 8080 -Action Allow -Profile Private,Domain -ErrorAction SilentlyContinue | Out-Null"""; \
    StatusMsg: "Adding firewall rule..."; \
    Flags: runhidden; \
    Tasks: firewall

; ── Mandatory: download ffmpeg + AI models + GeoNames (≋ 325 MB total). ──
; ffmpeg is required for video thumbnails / transcoding; the ONNX models
; back face/object recognition; the GeoNames dataset powers reverse
; geocoding. None of these have a working fallback so we always fetch.
Filename: "powershell.exe"; \
    Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\bin\fetch-assets.ps1"" -InstallDir ""{app}"" -DataDir ""{commonappdata}\SimplePhotos"""; \
    StatusMsg: "Downloading ffmpeg + AI models + GeoNames (~325 MB) ..."

; ── Optional: download CUDA 12 + cuDNN 9 runtime for GPU AI (gpu task). ────
; DLLs land next to the server binary in {app} so the ONNX CUDA provider can
; load them. Failure is non-fatal — the server falls back to CPU inference.
Filename: "powershell.exe"; \
    Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\bin\fetch-cuda-runtime.ps1"" -InstallDir ""{app}"""; \
    StatusMsg: "Downloading NVIDIA CUDA + cuDNN runtime for GPU acceleration (~0.6-1 GB) ..."; \
    Tasks: gpu

; ── Open the browser when finished ───────────────────────────────────────
Filename: "http://localhost:8080"; \
    Description: "Open Simple Photos in your browser"; \
    Flags: postinstall shellexec skipifsilent

[UninstallRun]
Filename: "{sys}\sc.exe"; Parameters: "stop SimplePhotos"; Flags: runhidden; RunOnceId: "StopSvc"
Filename: "{app}\bin\nssm.exe"; Parameters: "remove SimplePhotos confirm"; Flags: runhidden; RunOnceId: "RemoveSvc"
Filename: "powershell.exe"; \
    Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""Remove-NetFirewallRule -DisplayName 'SimplePhotos-Port-8080' -ErrorAction SilentlyContinue"""; \
    Flags: runhidden; RunOnceId: "RemoveFirewall"

[UninstallDelete]
; Application files only — preserve photo data in ProgramData.
; Operators must `rmdir /s /q "%ProgramData%\SimplePhotos"` themselves.
Type: filesandordirs; Name: "{app}"

[Code]
function InitializeSetup(): Boolean;
begin
  Result := True;
  // Reject Windows < 10 — the Rust binary uses APIs (e.g. WriteFileGather) that
  // are not present on 7/8.1. Fail fast instead of producing a confusing
  // "entry point not found" dialog at first run.
  if not (GetWindowsVersion >= $0A000000) then
  begin
    MsgBox('Simple Photos requires Windows 10 or later.', mbCriticalError, MB_OK);
    Result := False;
  end;
end;
