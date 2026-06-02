<#
.SYNOPSIS
    Simple Photos — Windows process watchdog / supervisor.

.DESCRIPTION
    Starts simple-photos-server.exe and restarts it automatically if it exits
    (crash, update, etc.).

    HOW TO FORCE-STOP THE SERVER PERMANENTLY:
      1. Open Task Manager -> Details tab.
      2. Find the "powershell.exe" entry whose Command Line references
         "watchdog.ps1" — End Task it.  This stops the watchdog loop.
      3. Then find "simple-photos-server.exe" and End Task it.

    Because step 2 kills the watchdog first, the server will NOT be restarted.
    Both processes are visible as separate entries in Task Manager -> Details.

    The $RestartDelaySec grace window means that if you only kill the server
    process (and leave the watchdog running), you have that many seconds to
    also kill the watchdog before it respawns the server.

    To prevent the server from starting on the next login as well, disable the
    "SimplePhotosServer" scheduled task:
        schtasks /Change /TN SimplePhotosServer /DISABLE

.NOTES
    This script is registered as the action for the "SimplePhotosServer"
    scheduled task created by install.ps1.  Do not move or rename it without
    re-running install.ps1 or updating the task manually.
#>

$ErrorActionPreference = 'Continue'   # non-fatal errors must not abort the loop

$ServerDir       = $PSScriptRoot
$ServerExe       = Join-Path $ServerDir 'target\release\simple-photos-server.exe'
$LogDir          = Join-Path $ServerDir 'data\logs'
$WatchdogLog     = Join-Path $LogDir    'watchdog.log'
$RestartDelaySec = 10

function Write-WatchdogLog {
    param([string]$Msg)
    $line = "[$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] $Msg"
    Write-Host $line
    try {
        if (-not (Test-Path $LogDir)) {
            New-Item -ItemType Directory -Path $LogDir -Force | Out-Null
        }
        Add-Content -Path $WatchdogLog -Value $line -Encoding UTF8
    } catch {
        # Log write failure is non-fatal — server may not have created data\logs yet
    }
}

if (-not (Test-Path $ServerExe)) {
    Write-WatchdogLog "FATAL: server executable not found at $ServerExe"
    exit 1
}

Write-WatchdogLog "Watchdog started. Executable: $ServerExe  Restart delay: ${RestartDelaySec}s"

while ($true) {
    Write-WatchdogLog "Starting server..."

    try {
        $proc = Start-Process `
            -FilePath         $ServerExe `
            -WorkingDirectory $ServerDir `
            -WindowStyle      Hidden `
            -PassThru
    } catch {
        Write-WatchdogLog "ERROR: could not start server — $_"
        Write-WatchdogLog "Waiting ${RestartDelaySec}s before retrying..."
        Start-Sleep -Seconds $RestartDelaySec
        continue
    }

    # Block until the server process terminates (crash, update, or manual kill).
    $proc | Wait-Process
    $code = $proc.ExitCode
    Write-WatchdogLog "Server exited (code $code)."

    # ── Grace window ────────────────────────────────────────────────────────
    # The server is now stopped.  We wait $RestartDelaySec seconds before
    # restarting it.  During this window a user who wants to permanently stop
    # the server can also kill THIS watchdog process (powershell.exe) from
    # Task Manager -> Details without the server coming back up.
    Write-WatchdogLog "Waiting ${RestartDelaySec}s before restart  (kill this watchdog process now to cancel)..."
    Start-Sleep -Seconds $RestartDelaySec

    Write-WatchdogLog "Restarting server..."
}
