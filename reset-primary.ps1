<#
.SYNOPSIS
    Windows equivalent of reset-primary.sh — reset the native (primary) Simple
    Photos server AND the Docker backup container: wipe DB + server-managed
    storage, rebuild web/android/server, reset TLS, and restart both.

.DESCRIPTION
    Development / test reset helper. Mirrors reset-primary.sh using Windows
    primitives (Windows Service / NSSM, Get-NetTCPConnection, Stop-Process).
    The Docker backup section is best-effort and is skipped automatically when
    Docker or the backup compose file is unavailable.

    SAFETY: an earlier incident wiped ~15 TB of user data. Every destructive
    operation MUST go through Invoke-SafePurgeManagedSubdirs (drive roots,
    system paths, shallow paths, and reparse points are refused) or the
    explicitly path-validated Docker backup block. Do not bypass these.
#>
[CmdletBinding()]
param(
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Run a native command (cargo / npm / gradle / docker) without letting its
# stderr output abort the script. Under `$ErrorActionPreference = 'Stop'`,
# PowerShell 5.1 raises a terminating "NativeCommandError" the moment a native
# program writes to stderr — and cargo, npm and gradle all stream ordinary
# progress to stderr. That made every build spuriously "fail". We relax the
# preference for the duration of the call and rely on the real exit code
# ($LASTEXITCODE) instead.
function Invoke-Native {
    param([Parameter(Mandatory)] [scriptblock]$Command)
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        # Stream command output (with stderr folded in) straight to the host so
        # the integer exit code is the ONLY thing returned. Returning the
        # command's stdout too would make `$code = Invoke-Native {...}` an array
        # of output lines, turning `if ($code -ne 0)` into a false failure.
        & $Command 2>&1 | Out-Host
    } finally {
        $ErrorActionPreference = $prev
    }
    return $LASTEXITCODE
}
$ServerDir = Join-Path $ScriptDir 'server'
$DockerDir = Join-Path $ScriptDir 'docker-instances'
$BackupDir = Join-Path $DockerDir 'simple-photos-backup-8081'
$BackupCompose = Join-Path $BackupDir 'docker-compose.yml'
$ServiceName = 'SimplePhotos'
$ProcessName = 'simple-photos-server'
$DefaultPort = 8080

$SafeManagedSubdirs = @(
    'blobs', 'metadata', 'logs', '.thumbnails', '.renders', '.tmp',
    '.web_previews', '.converted', 'uploads', '.ai_data', '.geo_cache'
)

# ============================================================================
# Safety helpers
# ============================================================================

function Abort([string]$Message) {
    Write-Host ''
    Write-Host '============================================================' -ForegroundColor Red
    Write-Host "FATAL SAFETY CHECK: $Message" -ForegroundColor Red
    Write-Host 'Aborting to protect your data.' -ForegroundColor Red
    Write-Host '============================================================' -ForegroundColor Red
    exit 1
}

function Test-SafeStorageRoot([string]$Root) {
    if ([string]::IsNullOrWhiteSpace($Root)) { return $false }
    if (-not [System.IO.Path]::IsPathRooted($Root)) { return $false }
    if (-not (Test-Path -LiteralPath $Root -PathType Container)) { return $false }
    if ($Root -match '[`$\r\n]') { return $false }

    $real = $null
    try { $real = (Resolve-Path -LiteralPath $Root -ErrorAction Stop).ProviderPath } catch { return $false }
    if ([string]::IsNullOrWhiteSpace($real)) { return $false }
    $real = $real.TrimEnd('\')

    if ($real -match '^[A-Za-z]:$') { return $false }

    $forbidden = @(
        $env:SystemRoot,
        $env:windir,
        (Join-Path $env:SystemDrive 'Windows'),
        $env:ProgramFiles,
        ${env:ProgramFiles(x86)},
        $env:ProgramData,
        $env:ProgramW6432,
        (Join-Path $env:SystemDrive 'Users'),
        $env:USERPROFILE,
        $env:PUBLIC
    ) | Where-Object { $_ } | ForEach-Object { $_.TrimEnd('\') }
    foreach ($f in $forbidden) {
        if ($real -ieq $f) { return $false }
    }

    $stripped = $real -replace '^[A-Za-z]:\\', ''
    if ($stripped -notmatch '\\') { return $false }

    return $true
}

function Invoke-SafePurgeManagedSubdirs {
    param(
        [Parameter(Mandatory)] [string]$Root,
        [Parameter(Mandatory)] [string[]]$Subdirs
    )
    if (-not (Test-SafeStorageRoot $Root)) {
        Abort "Refusing to clean storage root '$Root' — empty, missing, shallow, or a system path."
    }
    $realRoot = (Resolve-Path -LiteralPath $Root).ProviderPath.TrimEnd('\')

    foreach ($sd in $Subdirs) {
        if ([string]::IsNullOrWhiteSpace($sd) -or $sd -notmatch '^[A-Za-z0-9._-]+$' -or $sd -eq '.' -or $sd -eq '..') {
            Write-Host "  WARN: skipping invalid subdir name: '$sd'"
            continue
        }
        $target = Join-Path $realRoot $sd
        if (-not (Test-Path -LiteralPath $target)) { continue }

        $item = Get-Item -LiteralPath $target -Force
        if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
            Write-Host "  WARN: '$target' is a reparse point (junction/symlink) — leaving it alone."
            continue
        }
        if (-not ($item.PSIsContainer)) {
            Write-Host "  WARN: '$target' is not a directory — leaving it alone."
            continue
        }
        $realTarget = (Resolve-Path -LiteralPath $target).ProviderPath.TrimEnd('\')
        if (-not $realTarget.StartsWith("$realRoot\", [System.StringComparison]::OrdinalIgnoreCase)) {
            Write-Host "  WARN: '$target' resolves outside '$Root' — leaving it alone."
            continue
        }
        Write-Host "  Removing $target\ ..."
        try {
            Remove-Item -LiteralPath $realTarget -Recurse -Force -ErrorAction Stop
        } catch {
            Write-Host "  WARN: deletion of '$target' failed ($($_.Exception.Message))."
            Write-Host '        Please delete it manually.'
        }
    }
}

function Get-SafeStorageRoot([string]$ConfigFile) {
    if (-not (Test-Path -LiteralPath $ConfigFile -PathType Leaf)) { return '' }
    $rootHits = Select-String -LiteralPath $ConfigFile -Pattern '^\s*root\s*=' -AllMatches
    if (($rootHits | Measure-Object).Count -ne 1) { return '' }
    $line = $rootHits[0].Line
    if ($line -match '^\s*root\s*=\s*"([^"]*)"') { return $Matches[1] }
    return ''
}

# ============================================================================
# Port helpers
# ============================================================================

function Test-PortInUse([int]$Port) {
    try {
        $null = Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction Stop
        return $true
    } catch {
        return $false
    }
}

function Find-FreePort([int]$Start = $DefaultPort, [int]$Skip = -1) {
    $port = $Start
    for ($i = 0; $i -lt 100; $i++) {
        if ($port -ne $Skip -and -not (Test-PortInUse $port)) { return $port }
        Write-Host "  Port $port in use (or reserved), trying $($port + 1)..."
        $port++
    }
    Abort "No free port found after 100 attempts starting from $Start"
}

# ============================================================================
# Process / service + docker control
# ============================================================================

function Test-DockerAvailable {
    return [bool](Get-Command docker -ErrorAction SilentlyContinue)
}

function Stop-NativeServer {
    Write-Host 'Stopping native server...'
    # Intentionally do NOT stop the installed '$ServiceName' Windows service.
    # This dev reset launches the freshly-built native binary on its own (free)
    # port and leaves any production service running independently. Stopping it
    # here would take down the user's real instance without restarting it.
    $svc = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if ($svc -and $svc.Status -eq 'Running') {
        Write-Host "  (Leaving installed '$ServiceName' service running untouched.)"
    }
    # Resolve the service's backing process so we never try to kill it.
    $svcPid = $null
    try {
        $svcCim = Get-CimInstance Win32_Service -Filter "Name='$ServiceName'" -ErrorAction SilentlyContinue
        if ($svcCim -and $svcCim.ProcessId) { $svcPid = [int]$svcCim.ProcessId }
    } catch { }
    Get-Process -Name $ProcessName -ErrorAction SilentlyContinue |
        Where-Object { $_.Id -ne $svcPid } |
        Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
}

function Stop-BackupContainer {
    if ((Test-DockerAvailable) -and (Test-Path -LiteralPath $BackupCompose)) {
        Write-Host 'Stopping backup container...'
        & docker compose -f $BackupCompose down 2>$null | Out-Null
    }
}

# ============================================================================
# Build helpers
# ============================================================================

function Invoke-WebBuild {
    Write-Host 'Building web frontend...'
    $webDir = Join-Path $ScriptDir 'web'
    if (-not (Test-Path -LiteralPath $webDir)) { Write-Host "WARNING: $webDir not found — skipping web build"; return }
    if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
        Write-Host 'WARNING: npm not found — skipping web frontend build (using existing dist)'; return
    }
    Push-Location $webDir
    try {
        $code = Invoke-Native { npm run build }
        if ($code -ne 0) { throw "npm build exit $code" }
        Write-Host 'Web frontend built.'
    } catch {
        Write-Host 'WARNING: Web frontend build failed — continuing with existing dist'
    } finally { Pop-Location }
}

function Invoke-AndroidBuild {
    Write-Host 'Building Android APK...'
    $androidDir = Join-Path $ScriptDir 'android'
    $downloadsDir = Join-Path $ScriptDir 'downloads'
    if (-not (Test-Path -LiteralPath $androidDir)) { Write-Host "WARNING: $androidDir not found — skipping Android build"; return }
    if (-not (Get-Command java -ErrorAction SilentlyContinue)) { Write-Host 'WARNING: Java not found — skipping Android APK build'; return }
    New-Item -ItemType Directory -Path $downloadsDir -Force | Out-Null
    Push-Location $androidDir
    try {
        $code = Invoke-Native { .\gradlew.bat assembleDebug }
        if ($code -ne 0) { throw "gradlew exit $code" }
        $apk = Join-Path $androidDir 'app\build\outputs\apk\debug\app-debug.apk'
        if (Test-Path -LiteralPath $apk) {
            Copy-Item -LiteralPath $apk -Destination (Join-Path $downloadsDir 'simple-photos.apk') -Force
            Write-Host "Android APK copied to $downloadsDir\simple-photos.apk"
        } else {
            Write-Host "WARNING: APK not found at $apk after build"
        }
    } catch {
        Write-Host 'WARNING: Android APK build failed — continuing without APK'
    } finally { Pop-Location }
}

function Invoke-ServerBuild {
    Write-Host 'Building server binary...'
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Abort 'cargo not found on PATH — install Rust (https://rustup.rs) and retry.'
    }
    Push-Location $ServerDir
    try {
        $code = Invoke-Native { cargo build --release }
        if ($code -ne 0) { throw "cargo build exit $code" }
        Write-Host 'Server binary built.'
    } catch {
        Pop-Location
        Abort 'Server build failed. Aborting.'
    }
    Pop-Location

    $debugDir = Join-Path $ServerDir 'target\debug'
    if (Test-Path -LiteralPath $debugDir) {
        Write-Host 'Cleaning Rust debug build artifacts...'
        Remove-Item -LiteralPath $debugDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# ============================================================================
# Config patching + TLS reset
# ============================================================================

function Set-ConfigPort([string]$ConfigFile, [int]$Port) {
    if (-not (Test-Path -LiteralPath $ConfigFile -PathType Leaf)) { return }
    $text = Get-Content -LiteralPath $ConfigFile -Raw
    $oldPort = $null
    if ($text -match '(?m)^\s*port\s*=\s*(\d+)') { $oldPort = $Matches[1] }
    $text = $text -replace '(?m)^\s*port\s*=.*$', "port = $Port"
    if ($oldPort) { $text = $text -replace ":$oldPort\b", ":$Port" }
    Set-Content -LiteralPath $ConfigFile -Value $text -NoNewline
}

function Reset-TlsState([string]$ConfigFile) {
    if (-not (Test-Path -LiteralPath $ConfigFile -PathType Leaf)) { return }
    Write-Host 'Resetting TLS state in config.toml (enabled = false)...'
    try {
        $lines = Get-Content -LiteralPath $ConfigFile
        $out = New-Object System.Collections.Generic.List[string]
        $inTls = $false; $inTlsSub = $false; $sawEnabled = $false
        foreach ($line in $lines) {
            $m = [regex]::Match($line, '^\s*\[([^\]]+)\]\s*$')
            if ($m.Success) {
                $sect = $m.Groups[1].Value.Trim()
                if ($sect -eq 'tls') { $inTls = $true; $inTlsSub = $false; $out.Add($line); continue }
                if ($sect.StartsWith('tls.')) { $inTls = $false; $inTlsSub = $true; continue }
                if ($inTls -and -not $sawEnabled) { $out.Add('enabled = false'); $sawEnabled = $true }
                $inTls = $false; $inTlsSub = $false; $out.Add($line); continue
            }
            if ($inTlsSub) { continue }
            if ($inTls) {
                if ($line -match '^\s*enabled\s*=') { $out.Add('enabled = false'); $sawEnabled = $true; continue }
                if ($line -match '^\s*(cert_path|key_path|http_redirect_port|redirect_http)\s*=') { continue }
                $out.Add($line); continue
            }
            $line = $line -replace '(\bbase_url\s*=\s*")https://', '${1}http://'
            $out.Add($line)
        }
        if ($inTls -and -not $sawEnabled) { $out.Add('enabled = false') }
        $text = ($out -join "`r`n")
        if ($text -notmatch '(?m)^\s*\[tls\]\s*$') {
            $text = $text.TrimEnd() + "`r`n`r`n[tls]`r`nenabled = false`r`n"
        }
        Set-Content -LiteralPath $ConfigFile -Value $text
    } catch {
        Write-Host '  WARN: TLS reset failed — please flip [tls].enabled to false manually'
    }
    foreach ($sub in @('data\local_ca', 'data\acme')) {
        $dir = Join-Path $ServerDir $sub
        if (Test-Path -LiteralPath $dir) {
            Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue |
                Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

# ============================================================================
# Docker backup reset
# ============================================================================

function Reset-BackupContainer([int]$BackupPort) {
    if (-not (Test-Path -LiteralPath $BackupDir)) { return }
    Write-Host "Resetting Docker backup instance ($BackupDir)..."
    $backupData = Join-Path $BackupDir 'data'

    # Sanity: backup data must live under docker-instances — otherwise refuse.
    $realDockerDir = (Resolve-Path -LiteralPath $DockerDir -ErrorAction SilentlyContinue)
    if (-not $realDockerDir -or -not $backupData.StartsWith($realDockerDir.ProviderPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        Abort "Backup data path '$backupData' is outside the project — refusing to wipe."
    }

    if (Test-Path -LiteralPath $backupData) {
        $bkDb = Join-Path $backupData 'db'
        if (Test-Path -LiteralPath $bkDb) {
            Get-ChildItem -LiteralPath $bkDb -Force -ErrorAction SilentlyContinue |
                Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
        }
        $bkStorage = Join-Path $backupData 'storage'
        if (Test-Path -LiteralPath $bkStorage) {
            # Path already validated under docker-instances above.
            Remove-Item -LiteralPath $bkStorage -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    New-Item -ItemType Directory -Path (Join-Path $backupData 'db') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $backupData 'storage') -Force | Out-Null
    Write-Host '  backup data wiped'

    if (Test-Path -LiteralPath $BackupCompose) {
        $compose = Get-Content -LiteralPath $BackupCompose -Raw
        $containerPort = 3000
        $pm = [regex]::Match($compose, '-\s*"?\d+:(\d+)"?')
        if ($pm.Success) { $containerPort = [int]$pm.Groups[1].Value }
        $compose = [regex]::Replace($compose, '(-\s*"?)\d+(:\d+"?)', "`${1}$BackupPort`${2}")
        Set-Content -LiteralPath $BackupCompose -Value $compose -NoNewline

        $backupConfig = Join-Path $BackupDir 'config.toml'
        if (Test-Path -LiteralPath $backupConfig) {
            $bc = Get-Content -LiteralPath $backupConfig -Raw
            $om = [regex]::Match($bc, '(?m)^base_url\s*=.*:(\d+)')
            if ($om.Success) {
                $oldBkPort = $om.Groups[1].Value
                $bc = $bc -replace ":$oldBkPort\b", ":$BackupPort"
                Set-Content -LiteralPath $backupConfig -Value $bc -NoNewline
            }
        }
        Write-Host "  Backup mapped to host port $BackupPort (container port $containerPort)"
    }

    if ((Test-DockerAvailable) -and (Test-Path -LiteralPath $BackupCompose)) {
        $netCode = Invoke-Native { docker network inspect simple-photos-net 2>$null | Out-Null }
        if ($netCode -ne 0) { Invoke-Native { docker network create simple-photos-net 2>$null | Out-Null } | Out-Null }

        Write-Host 'Building backup container image (no-cache, may take a few minutes)...'
        $bCode = Invoke-Native { docker compose -f $BackupCompose build --no-cache }
        if ($bCode -ne 0) { Write-Host 'WARNING: docker compose build failed for backup' }
        Write-Host 'Starting backup container...'
        $uCode = Invoke-Native { docker compose -f $BackupCompose up -d --force-recreate }
        if ($uCode -ne 0) { Write-Host 'WARNING: docker compose up failed for backup' }
        Invoke-Native { docker image prune -f 2>$null | Out-Null } | Out-Null

        Write-Host -NoNewline "Waiting for backup on :$BackupPort"
        $bkReady = $false
        for ($i = 0; $i -lt 30; $i++) {
            foreach ($path in @('/api/health', '/api/setup/status')) {
                try {
                    $r = Invoke-WebRequest -Uri "http://localhost:$BackupPort$path" -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
                    if ($r.StatusCode -eq 200) { $bkReady = $true; break }
                } catch { }
            }
            if ($bkReady) { break }
            Write-Host -NoNewline '.'
            Start-Sleep -Seconds 1
        }
        if ($bkReady) { Write-Host " ready!"; Write-Host "  backup is running at http://localhost:$BackupPort" }
        else {
            Write-Host ''
            Write-Host '  Warning: backup did not become healthy in time'
            & docker compose -f $BackupCompose logs --tail=30 2>$null
        }
    } elseif (-not (Test-DockerAvailable)) {
        Write-Host '  Skipping backup container — docker not installed'
    }
}

# ============================================================================
# Restart native server
# ============================================================================

function Start-NativeServer([int]$Port) {
    $logFile = Join-Path ([System.IO.Path]::GetTempPath()) 'simple-photos-server.log'
    $errFile = "$logFile.err"
    Write-Host "Starting server (log: $logFile)..."
    # Clear stale logs so the readiness check reflects this boot only (and to
    # surface a lingering lock from a previous instance immediately).
    Remove-Item -LiteralPath $logFile, $errFile -Force -ErrorAction SilentlyContinue
    # This is a DEV/TEST reset: it just rebuilt server\target\release and wiped
    # the dev database + storage, so it must launch *that* freshly-built binary
    # on the dev port we picked. Do NOT defer to the installed Windows service:
    # the service runs its own production binary against its own config
    # (%ProgramData%\SimplePhotos, typically port 8080), so starting it would
    # ignore both the rebuild and the dynamic dev port — and on an elevated
    # service we usually can't control its lifecycle from a normal shell anyway.
    $svc = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if ($svc -and $svc.Status -eq 'Running') {
        Write-Host "NOTE: An installed '$ServiceName' service is running independently" `
            '(its own config/port). This dev reset does NOT touch it; it launches the'
        Write-Host "      freshly-built binary on port $Port instead."
    }
    $exe = Join-Path $ServerDir 'target\release\simple-photos-server.exe'
    if (-not (Test-Path -LiteralPath $exe)) { Abort "Server binary not found at $exe" }
    $proc = Start-Process -FilePath $exe -WorkingDirectory $ServerDir `
        -RedirectStandardOutput $logFile -RedirectStandardError $errFile `
        -WindowStyle Hidden -PassThru

    Write-Host -NoNewline 'Waiting for server'
    $ready = $false
    # Generous: first boot loads ONNX AI models before serving. On a CPU-only
    # host this can take the better part of a minute — and longer when the disk
    # and CPU are still busy flushing the builds that ran moments ago. Allow up
    # to ~120s, but bail out early if the process actually dies.
    for ($i = 0; $i -lt 120; $i++) {
        if ($proc -and $proc.HasExited) {
            Write-Host ''
            Write-Host "ERROR: Server process exited early (code $($proc.ExitCode)) after ${i}s."
            if (Test-Path -LiteralPath $logFile) { Get-Content -LiteralPath $logFile -Tail 20 }
            if (Test-Path -LiteralPath $errFile) { Get-Content -LiteralPath $errFile -Tail 20 }
            return
        }
        try {
            # Probe 127.0.0.1 (IPv4) explicitly. The server binds 0.0.0.0 (IPv4
            # only); 'localhost' resolves to ::1 (IPv6) first on Windows, so a
            # localhost probe can spuriously time out even when the server is up.
            $r = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/api/setup/status" -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
            if ($r.StatusCode -eq 200) { $ready = $true; break }
        } catch { }
        Write-Host -NoNewline '.'
        Start-Sleep -Seconds 1
    }
    if ($ready) { Write-Host ' ready!' }
    elseif ($proc -and -not $proc.HasExited) {
        Write-Host ''
        Write-Host "NOTE: Server process is running (pid $($proc.Id)) but not answering on :$Port yet."
        Write-Host '      It is most likely still loading AI models — give it another moment.'
        Write-Host "      Watch progress with: Get-Content '$logFile' -Wait -Tail 20"
    }
    else {
        Write-Host ''
        Write-Host "WARNING: Server may have failed to start. Check $logFile"
        if (Test-Path -LiteralPath $logFile) { Get-Content -LiteralPath $logFile -Tail 20 }
    }
}

# ============================================================================
# Main
# ============================================================================

Write-Host '=== Reset Primary + Docker Backup (Windows) ==='

Stop-NativeServer
Stop-BackupContainer

# Native (primary) takes priority — first free port. Backup gets the next.
$ServerPort = Find-FreePort $DefaultPort
$BackupPort = Find-FreePort $DefaultPort $ServerPort

Invoke-WebBuild
Invoke-AndroidBuild
Invoke-ServerBuild

# ── Wipe database ──
Write-Host 'Wiping database...'
$ConfigFile = Join-Path $ServerDir 'config.toml'
$StorageRoot = Get-SafeStorageRoot $ConfigFile
$dbDir = Join-Path $ServerDir 'data\db'
if (Test-Path -LiteralPath $dbDir) {
    Get-ChildItem -LiteralPath $dbDir -Force -ErrorAction SilentlyContinue |
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
}

# ── Clean server-managed storage ──
Write-Host 'Cleaning internal storage (server-managed dirs only)...'
$internalStorage = Join-Path $ServerDir 'data\storage'
if (Test-Path -LiteralPath $internalStorage) {
    Invoke-SafePurgeManagedSubdirs -Root $internalStorage -Subdirs $SafeManagedSubdirs
}
if ($StorageRoot -and (Test-Path -LiteralPath $StorageRoot)) {
    Write-Host "Cleaning storage root subdirectories in: $StorageRoot"
    Invoke-SafePurgeManagedSubdirs -Root $StorageRoot -Subdirs $SafeManagedSubdirs
    Write-Host 'Original photos preserved.'
} else {
    Write-Host 'Notice: No storage root configured (or unreadable) — skipping external cleanup.'
}
Write-Host 'Data cleared.'

# ── Docker backup ──
Reset-BackupContainer $BackupPort

# ── Patch port + reset TLS, then restart primary ──
Set-ConfigPort $ConfigFile $ServerPort
Reset-TlsState $ConfigFile
Start-NativeServer $ServerPort

Write-Host ''
Write-Host '╔══════════════════════════════════════════════════╗'
Write-Host '║          Reset complete — server addresses       ║'
Write-Host '╠══════════════════════════════════════════════════╣'
Write-Host ("║  Primary :  http://localhost:{0,-20}║" -f $ServerPort)
if ((Test-Path -LiteralPath $BackupDir) -and (Test-DockerAvailable)) {
    Write-Host ("║  Backup  :  http://localhost:{0,-20}║" -f $BackupPort)
}
Write-Host '╚══════════════════════════════════════════════════╝'
Write-Host ''
