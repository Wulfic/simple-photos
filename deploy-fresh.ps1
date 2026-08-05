<#
.SYNOPSIS
    Windows-side "fresh deploy" orchestrator for the live CT132 Docker instance.

.DESCRIPTION
    The authoritative deploy logic lives in deploy-fresh.sh, which runs *inside*
    CT132 (the LXC that owns the Docker instance) and does the git reset, web
    build, `docker compose build`, guard-railed storage wipe, asset provisioning
    and health check. This script is the thin Windows client for that flow:

        1. (optional) build the web bundle locally as a fail-fast sanity gate
        2. (optional) build the Android APK locally — CT132 has no Android SDK,
           so this is the ONLY way to ship a freshly-compiled debug APK; without
           it deploy-fresh.sh can merely fetch a GitHub *release* APK
        3. push the target branch to origin (the box deploys origin/<branch>)
        4. Posh-SSH into Proxmox (192.168.86.87) and, via `pct exec 132`:
             - stage the locally-built APK into <repo>/downloads/simple-photos.apk
             - run deploy-fresh.sh --branch <branch> --yes [flags]
        5. poll the container's /api/health + /api/setup/status from Windows

    Topology / secrets are read from .env (SUDO_PASSWORD / PROXMOX_PASSWORD /
    ROOT_PASSWORD for the PVE root login). Nothing here bypasses the box-side
    guard-rails — the destructive wipe is entirely owned by deploy-fresh.sh.

.PARAMETER Branch
    Branch to deploy (default: dev). Pushed to origin, then the box hard-resets
    to origin/<branch>.

.PARAMETER NoWipe
    Pass-through to deploy-fresh.sh --no-wipe (update + rebuild only, keep data).

.PARAMETER WipeTakeout
    Pass-through to deploy-fresh.sh --wipe-takeout (ALSO delete import sources).

.PARAMETER SkipAssets
    Pass-through to deploy-fresh.sh --skip-assets (don't (re)provision models/geo/apk).

.PARAMETER SkipWebBuild
    Skip the local web build sanity gate (the box rebuilds web from git regardless).

.PARAMETER SkipAndroid
    Skip building + shipping the local APK (the box will fall back to a release APK).

.PARAMETER SkipPush
    Do NOT `git push` — deploy whatever is already on origin/<branch>.

.PARAMETER Yes
    Skip the local wipe confirmation prompt (equivalent to answering "y").

.PARAMETER DryRun
    Print every remote command instead of executing it.
#>
[CmdletBinding()]
param(
    [string]$Branch      = 'dev',
    [string]$Instance    = 'simple-photos',
    [string]$PveHost     = '192.168.86.87',
    [string]$PveUser     = 'root',
    [int]   $Ctid        = 132,
    [string]$RemoteRepo  = '/opt/simple-photos',
    [string]$ServerHost  = '192.168.86.132',
    [int]   $Port        = 8080,
    [switch]$NoWipe,
    [switch]$WipeTakeout,
    [switch]$SkipAssets,
    [switch]$SkipWebBuild,
    [switch]$SkipAndroid,
    [switch]$SkipPush,
    [switch]$Yes,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$ScriptDir = $PSScriptRoot

# ============================================================================
# Pretty output (mirrors deploy-fresh.sh)
# ============================================================================
function Info([string]$m) { Write-Host "i  $m" -ForegroundColor Blue }
function Ok([string]$m)   { Write-Host "OK $m" -ForegroundColor Green }
function Warn([string]$m) { Write-Host "!  $m" -ForegroundColor Yellow }
function Step([string]$m) { Write-Host ''; Write-Host "=== $m ===" -ForegroundColor White -BackgroundColor DarkBlue }
function Abort([string]$m) {
    Write-Host ''
    Write-Host "FATAL: $m" -ForegroundColor Red
    Write-Host 'Aborting.' -ForegroundColor Red
    exit 1
}

# Run a native command (npm / gradle / git) without its stderr progress output
# tripping $ErrorActionPreference='Stop'. See reset-server.ps1 for the full
# rationale — PS 5.1 wraps every native stderr line in a NativeCommandError and
# renders it as a red failure banner even on exit 0. We stringify each item and
# rely on $LASTEXITCODE only.
function Invoke-Native {
    param([Parameter(Mandatory)] [scriptblock]$Command)
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Command 2>&1 | ForEach-Object { Write-Host ($_.ToString()) }
    } finally {
        $ErrorActionPreference = $prev
    }
    return $LASTEXITCODE
}

# ============================================================================
# .env loading (only the PVE root password is needed here)
# ============================================================================
function Get-DotEnv([string]$File) {
    $map = @{}
    if (-not (Test-Path -LiteralPath $File)) { return $map }
    foreach ($line in (Get-Content -LiteralPath $File)) {
        if ($line -match '^\s*#' -or $line -notmatch '=') { continue }
        $k, $v = $line -split '=', 2
        $k = $k.Trim()
        $v = $v.Trim().Trim('"').Trim("'")
        if ($k) { $map[$k] = $v }
    }
    return $map
}

function Get-PveCredential {
    $env = Get-DotEnv (Join-Path $ScriptDir '.env')
    $pw = $null
    foreach ($key in 'SUDO_PASSWORD', 'PROXMOX_PASSWORD', 'ROOT_PASSWORD') {
        if ($env.ContainsKey($key) -and -not [string]::IsNullOrWhiteSpace($env[$key])) {
            $pw = $env[$key]; break
        }
    }
    if (-not $pw) { Abort "No PVE password in .env (looked for SUDO_PASSWORD / PROXMOX_PASSWORD / ROOT_PASSWORD)." }
    $sec = ConvertTo-SecureString $pw -AsPlainText -Force
    return (New-Object System.Management.Automation.PSCredential($PveUser, $sec))
}

# ============================================================================
# Posh-SSH helpers
# ============================================================================
function Initialize-PoshSSH {
    if (Get-Module -ListAvailable -Name Posh-SSH) { Import-Module Posh-SSH; return }
    Warn 'Posh-SSH module not installed — installing for current user…'
    try {
        Install-Module -Name Posh-SSH -Scope CurrentUser -Force -AllowClobber -ErrorAction Stop
        Import-Module Posh-SSH
        Ok 'Posh-SSH installed.'
    } catch {
        Abort "Could not install Posh-SSH ($($_.Exception.Message)). Install it manually: Install-Module Posh-SSH -Scope CurrentUser"
    }
}

# Run a bash payload on the PVE host. The payload is UTF8→base64 encoded and
# piped through `base64 -d | bash` to dodge every layer of SSH/pct/bash quoting
# (the same trick the deploy memory documents for pct exec). Streams remote
# output to the host and returns the remote exit status.
function Invoke-Pve {
    param(
        [Parameter(Mandatory)] [int]$SessionId,
        [Parameter(Mandatory)] [string]$Bash,
        [int]$TimeOut = 3600,
        [switch]$AllowFail
    )
    $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Bash))
    $cmd = "echo $b64 | base64 -d | bash"
    if ($DryRun) {
        Write-Host "--- DRYRUN would run on PVE:$PveHost ---" -ForegroundColor DarkGray
        Write-Host $Bash -ForegroundColor DarkGray
        return 0
    }
    $r = Invoke-SSHCommand -SessionId $SessionId -Command $cmd -TimeOut $TimeOut
    if ($r.Output)      { $r.Output      | ForEach-Object { Write-Host $_ } }
    if ($r.ExitStatus -ne 0) {
        if ($r.Error) { $r.Error | ForEach-Object { Write-Host $_ -ForegroundColor Red } }
        if (-not $AllowFail) { Abort "Remote command failed (exit $($r.ExitStatus))." }
    }
    return $r.ExitStatus
}

# ============================================================================
# Local build stages
# ============================================================================
function Invoke-WebBuild {
    Step 'Build web frontend (local sanity gate)'
    $webDir = Join-Path $ScriptDir 'web'
    if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
        Warn 'npm not found — skipping local web build (box rebuilds web from git regardless).'
        return
    }
    Push-Location $webDir
    try {
        $code = Invoke-Native { npm run build }
        if ($code -ne 0) { Abort "Local web build failed (exit $code). Fix it before deploying — the box would fail the same way." }
        Ok 'web/dist builds clean.'
    } finally { Pop-Location }
}

# Returns the path to the built APK, or $null.
function Invoke-AndroidBuild {
    Step 'Build Android APK (local)'
    $androidDir = Join-Path $ScriptDir 'android'
    $gradlew = Join-Path $androidDir 'gradlew.bat'
    if (-not (Test-Path -LiteralPath $gradlew)) {
        Warn "gradlew.bat not found at $gradlew — skipping APK build."
        return $null
    }
    if (-not (Get-Command java -ErrorAction SilentlyContinue)) {
        Warn 'java not found on PATH — skipping APK build (box will fall back to a release APK).'
        return $null
    }
    Push-Location $androidDir
    try {
        $code = Invoke-Native { .\gradlew.bat assembleDebug }
        if ($code -ne 0) { Abort "gradlew assembleDebug failed (exit $code)." }
    } finally { Pop-Location }
    $apk = Join-Path $androidDir 'app\build\outputs\apk\debug\app-debug.apk'
    if (-not (Test-Path -LiteralPath $apk)) { Abort "APK not found at $apk after a successful build." }
    Ok "APK built: $apk ($([math]::Round((Get-Item $apk).Length / 1MB, 1)) MB)"
    return $apk
}

# ============================================================================
# Main
# ============================================================================
Write-Host '========================================================' -ForegroundColor Cyan
Write-Host '  Simple Photos — Fresh Deploy (Windows -> CT132)' -ForegroundColor Cyan
Write-Host '========================================================' -ForegroundColor Cyan

$DoWipe = -not $NoWipe
Info "Branch     : $Branch"
Info "Instance   : $Instance"
Info "PVE        : $PveUser@$PveHost  (CT $Ctid, repo $RemoteRepo)"
Info "Target     : http://${ServerHost}:$Port"
Info "Fresh wipe : $DoWipe   (wipe Takeout: $([bool]$WipeTakeout))"

# --- Local wipe confirmation (remote pct exec is non-interactive) ---
if ($DoWipe -and -not $Yes -and -not $DryRun) {
    Write-Host ''
    Warn 'This will DELETE the database and server-managed storage dirs on the LIVE instance.'
    Warn 'Originals / import sources (Takeout/) are preserved unless -WipeTakeout is set.'
    if ($WipeTakeout) { Warn '  + Takeout/ (import sources) WILL be removed — you passed -WipeTakeout.' }
    $reply = Read-Host '  Proceed with fresh wipe? [y/N]'
    if ($reply -notmatch '^[Yy]$') { Abort 'User declined.' }
}

# --- Pre-flight ---
Step 'Pre-flight'
if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Abort 'git not found on PATH.' }
Initialize-PoshSSH
$cred = Get-PveCredential
Ok 'Posh-SSH ready; PVE credential loaded from .env.'

# Warn loudly if the working tree has uncommitted changes: the box hard-resets
# to origin/<branch>, so anything not pushed will NOT be deployed.
$dirty = (git -C $ScriptDir status --porcelain)
if ($dirty -and -not $SkipPush) {
    Warn "Working tree has uncommitted changes — the box deploys origin/$Branch, so these will NOT ship until committed + pushed."
}

# --- Local builds ---
if (-not $SkipWebBuild) { Invoke-WebBuild } else { Info 'Skipping local web build (-SkipWebBuild).' }
$apk = $null
if (-not $SkipAndroid)  { $apk = Invoke-AndroidBuild } else { Info 'Skipping Android build (-SkipAndroid).' }

# --- Push branch ---
if (-not $SkipPush) {
    Step "Push branch -> origin/$Branch"
    $code = Invoke-Native { git -C $ScriptDir push origin $Branch }
    if ($code -ne 0) { Abort "git push failed (exit $code)." }
    Ok "origin/$Branch updated."
} else {
    Info 'Skipping git push (-SkipPush) — deploying whatever is on origin.'
}

# --- Open SSH session to PVE ---
Step "Connect to PVE ($PveUser@$PveHost)"
$session = $null
if (-not $DryRun) {
    $session = New-SSHSession -ComputerName $PveHost -Credential $cred -AcceptKey -ErrorAction Stop
    Ok "SSH session $($session.SessionId) established."
}
$sid = if ($session) { $session.SessionId } else { -1 }

try {
    # --- Stage the locally-built APK into the container's downloads/ ---
    if ($apk) {
        Step 'Stage APK into container'
        $tmpRemote = "/tmp/sp-deploy-$([DateTime]::Now.ToString('yyyyMMdd-HHmmss')).apk"
        if ($DryRun) {
            Write-Host "--- DRYRUN scp $apk -> ${PveHost}:$tmpRemote ; pct push $Ctid -> $RemoteRepo/downloads/simple-photos.apk" -ForegroundColor DarkGray
        } else {
            # Set-SCPItem keeps the source filename, so target a directory then move.
            Set-SCPItem -ComputerName $PveHost -Credential $cred -Path $apk -Destination '/tmp' -AcceptKey -ErrorAction Stop
            $scpName = "/tmp/$([System.IO.Path]::GetFileName($apk))"  # /tmp/app-debug.apk
            $stage = @"
set -e
mv -f '$scpName' '$tmpRemote'
pct exec $Ctid -- mkdir -p '$RemoteRepo/downloads'
pct push $Ctid '$tmpRemote' '$RemoteRepo/downloads/simple-photos.apk'
rm -f '$tmpRemote'
echo "APK staged -> $RemoteRepo/downloads/simple-photos.apk"
"@
            [void](Invoke-Pve -SessionId $sid -Bash $stage -TimeOut 300)
        }
        Ok 'APK staged for provisioning.'
    }

    # --- Run the box-side deploy-fresh.sh ---
    Step 'Run deploy-fresh.sh on CT132'
    # Single-quote values: this whole string is nested inside the outer
    # `bash -lc "..."` double quotes, so double-quoting here would close it early.
    $flags = @("--branch '$Branch'", "--instance '$Instance'", '--yes')
    if ($NoWipe)      { $flags += '--no-wipe' }
    if ($WipeTakeout) { $flags += '--wipe-takeout' }
    if ($SkipAssets)  { $flags += '--skip-assets' }
    $flagStr = $flags -join ' '

    # deploy-fresh.sh must run INSIDE the container that owns docker/compose.
    $deploy = @"
set -e
pct exec $Ctid -- bash -lc "cd '$RemoteRepo' && chmod +x ./deploy-fresh.sh 2>/dev/null; ./deploy-fresh.sh $flagStr"
"@
    Info "remote: deploy-fresh.sh $flagStr"
    [void](Invoke-Pve -SessionId $sid -Bash $deploy -TimeOut 5400)
    Ok 'Remote deploy-fresh.sh finished.'
}
finally {
    if ($session) { Remove-SSHSession -SessionId $sid | Out-Null }
}

# --- Health check from Windows ---
Step 'Verify (from Windows)'
if ($DryRun) { Info 'DryRun — skipping health check.'; exit 0 }

$healthUrl = "http://${ServerHost}:$Port/api/health"
$statusUrl = "http://${ServerHost}:$Port/api/setup/status"
Write-Host -NoNewline "Waiting for $healthUrl "
$ready = $false
for ($i = 0; $i -lt 90; $i++) {
    try {
        $r = Invoke-WebRequest -Uri $healthUrl -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
        if ($r.StatusCode -eq 200) { $ready = $true; break }
    } catch { }
    Write-Host -NoNewline '.'
    Start-Sleep -Seconds 2
}
Write-Host ''
if ($ready) {
    Ok 'Healthy.'
    try {
        $s = Invoke-WebRequest -Uri $statusUrl -UseBasicParsing -TimeoutSec 5 -ErrorAction Stop
        Info "setup/status: $($s.Content)"
    } catch { Warn "Could not read setup/status: $($_.Exception.Message)" }
} else {
    Warn "Did not become healthy at $healthUrl within ~3 min."
    Warn 'Check the container logs on CT132: pct exec 132 -- docker logs ' + $Instance
    exit 1
}

Write-Host ''
Write-Host "Deploy complete.  ->  http://${ServerHost}:$Port" -ForegroundColor Green
if ($DoWipe) { Info 'Fresh setup: open the URL and complete the first-run wizard. APK is on the download page.' }
