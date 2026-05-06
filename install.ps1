<#
.SYNOPSIS
    Simple Photos — Install & Setup Script (Windows)

.DESCRIPTION
    Supports both Docker and native installations with auto-port detection,
    CLI parameters, and interactive mode.

.PARAMETER Mode
    Installation mode: "native" or "docker"

.PARAMETER Port
    Starting port number (auto-increments if busy). Default: 8080

.PARAMETER Name
    Instance name (for Docker containers). Default: auto-generated

.PARAMETER StoragePath
    Path to photo storage directory

.PARAMETER AdminUser
    Admin username (skip interactive prompt)

.PARAMETER AdminPass
    Admin password (skip interactive prompt)

.PARAMETER NoBuildAndroid
    Skip Android APK build prompt

.PARAMETER NoStart
    Don't start the server after install

.PARAMETER SkipModels
    Don't download AI ONNX models or the GeoNames dataset

.PARAMETER Yes
    Auto-accept all prompts

.EXAMPLE
    .\install.ps1
    .\install.ps1 -Mode native -Port 8080
    .\install.ps1 -Mode docker -Port 8080
#>

[CmdletBinding()]
param(
    [ValidateSet("native", "docker", "")]
    [string]$Mode = "",

    [int]$Port = 0,

    [string]$Name = "",
    [string]$StoragePath = "",
    [string]$AdminUser = "",
    [string]$AdminPass = "",
    [string]$LetsEncryptDomain = "",
    [string]$LetsEncryptEmail = "",
    [switch]$LetsEncryptStaging,
    [switch]$LetsEncryptAgreeTos,
    [switch]$LocalCa,
    [switch]$NoBuildAndroid,
    [switch]$NoStart,
    [switch]$SkipModels,
    [switch]$Yes
)

# ══════════════════════════════════════════════════════════════════════════════
# Strict mode & helpers
# ══════════════════════════════════════════════════════════════════════════════
$ErrorActionPreference = "Stop"
$DefaultPort = 8080
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $ScriptDir

function Write-Info    { param([string]$Msg) Write-Host "  i " -ForegroundColor Blue -NoNewline; Write-Host $Msg }
function Write-Ok      { param([string]$Msg) Write-Host "  √ " -ForegroundColor Green -NoNewline; Write-Host $Msg }
function Write-Warn    { param([string]$Msg) Write-Host "  ! " -ForegroundColor Yellow -NoNewline; Write-Host $Msg }
function Write-Err     { param([string]$Msg) Write-Host "  X " -ForegroundColor Red -NoNewline; Write-Host $Msg }
function Write-Step    { param([string]$Msg) Write-Host "`n--- $Msg ---`n" -ForegroundColor Cyan }

# Download a single URL to $OutPath, skipping if the file already exists.
function Invoke-FileDownload {
    param([string]$Url, [string]$OutPath)
    if ((Test-Path $OutPath) -and (Get-Item $OutPath).Length -gt 0) {
        Write-Info "[dl] $(Split-Path $OutPath -Leaf) already present — skipping"
        return
    }
    Write-Info "[dl] Downloading $(Split-Path $OutPath -Leaf)…"
    $part = "$OutPath.part"
    try {
        Invoke-WebRequest -Uri $Url -OutFile $part -UseBasicParsing
        if (-not (Test-Path $part) -or (Get-Item $part).Length -eq 0) {
            throw "Download produced an empty file."
        }
        Move-Item -Path $part -Destination $OutPath -Force
        $size = [math]::Round((Get-Item $OutPath).Length / 1MB, 1)
        Write-Info "[dl]   → ${size} MB"
    } catch {
        Remove-Item $part -ErrorAction SilentlyContinue
        throw "Failed to download $(Split-Path $OutPath -Leaf): $_"
    }
}

# Download ONNX models for AI face/object recognition.
# Models: SCRFD face detector, ArcFace embeddings, UltraFace fallback,
#         MobileNetV2 object classifier (all Apache-2.0 / MIT-licensed).
function Download-AiModels {
    param([string]$Target)
    if (-not (Test-Path $Target)) { New-Item -ItemType Directory -Path $Target -Force | Out-Null }

    Invoke-FileDownload `
        -Url "https://huggingface.co/immich-app/buffalo_l/resolve/main/detection/model.onnx" `
        -OutPath (Join-Path $Target "det_10g.onnx")
    Invoke-FileDownload `
        -Url "https://huggingface.co/immich-app/buffalo_l/resolve/main/recognition/model.onnx" `
        -OutPath (Join-Path $Target "w600k_r50.onnx")
    Invoke-FileDownload `
        -Url "https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/raw/master/models/onnx/version-RFB-320.onnx" `
        -OutPath (Join-Path $Target "ultraface-RFB-320.onnx")
    Invoke-FileDownload `
        -Url "https://github.com/onnx/models/raw/refs/heads/main/validated/vision/classification/mobilenet/model/mobilenetv2-12.onnx" `
        -OutPath (Join-Path $Target "mobilenetv2-12.onnx")

    Write-Info "[ai] Models present in $Target:"
    Get-ChildItem $Target | ForEach-Object { Write-Info "  $($_.Name)  ($([math]::Round($_.Length/1MB,1)) MB)" }
}

# Download the GeoNames cities500 dataset for offline reverse geocoding.
# License: CC BY 4.0 — attribute geonames.org.
function Download-GeoData {
    param([string]$Target)
    $dir = Split-Path $Target -Parent
    if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }

    $zipPath = Join-Path $env:TEMP "cities500_$([System.IO.Path]::GetRandomFileName()).zip"
    try {
        Write-Info "[geo] Downloading GeoNames cities500 dataset…"
        Invoke-WebRequest `
            -Uri "https://download.geonames.org/export/dump/cities500.zip" `
            -OutFile $zipPath -UseBasicParsing

        Write-Info "[geo] Extracting…"
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $zip = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
        try {
            $entry = $zip.Entries | Where-Object { $_.Name -eq "cities500.txt" } | Select-Object -First 1
            if (-not $entry) { throw "cities500.txt not found in archive" }
            [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $Target, $true)
        } finally {
            $zip.Dispose()
        }
        $lines = (Get-Content $Target | Measure-Object -Line).Lines
        Write-Info "[geo] Done — $lines cities written to $Target"
    } finally {
        Remove-Item $zipPath -ErrorAction SilentlyContinue
    }
}

function Test-PortInUse {
    param([int]$PortNumber)
    $listener = $null
    try {
        $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, $PortNumber)
        $listener.Start()
        $listener.Stop()
        return $false
    } catch {
        return $true
    } finally {
        if ($listener) { try { $listener.Stop() } catch {} }
    }
}

function Find-AvailablePort {
    param([int]$StartPort)
    $p = $StartPort
    $max = 100
    for ($i = 0; $i -lt $max; $i++) {
        if (-not (Test-PortInUse $p)) { return $p }
        Write-Warn "Port $p is in use, trying $($p + 1)..."
        $p++
    }
    throw "No available port found after $max attempts (starting from $StartPort)"
}

function Read-Prompt {
    param([string]$Question, [string]$Default = "")
    if ($Yes -and $Default) { return $Default }
    if ($Default) {
        $reply = Read-Host "  $Question [$Default]"
        if ([string]::IsNullOrWhiteSpace($reply)) { return $Default }
        return $reply
    } else {
        return Read-Host "  $Question"
    }
}

function Read-YesNo {
    param([string]$Question, [string]$Default = "Y")
    if ($Yes) { return ($Default -eq "Y") }
    $hint = if ($Default -eq "Y") { "[Y/n]" } else { "[y/N]" }
    $reply = Read-Host "  $Question $hint"
    if ([string]::IsNullOrWhiteSpace($reply)) { $reply = $Default }
    return ($reply -match "^[Yy]")
}

function New-SecureKey {
    $bytes = [byte[]]::new(32)
    [System.Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    return ($bytes | ForEach-Object { $_.ToString("x2") }) -join ""
}

function Test-CommandExists {
    param([string]$Cmd)
    return [bool](Get-Command $Cmd -ErrorAction SilentlyContinue)
}

# ══════════════════════════════════════════════════════════════════════════════
# Banner
# ══════════════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host "  +================================================+" -ForegroundColor White
Write-Host "  |         Simple Photos Installer (Windows)       |" -ForegroundColor White
Write-Host "  |    Self-hosted E2E encrypted photo library      |" -ForegroundColor White
Write-Host "  +================================================+" -ForegroundColor White
Write-Host ""

# ══════════════════════════════════════════════════════════════════════════════
# Step 1: Installation mode
# ══════════════════════════════════════════════════════════════════════════════
Write-Step "Step 1/7: Installation mode"

if (-not $Mode) {
    Write-Host "  How would you like to install Simple Photos?" -ForegroundColor White
    Write-Host ""
    Write-Host "  1) Native  - build from source (requires Rust & Node.js)" -ForegroundColor Cyan
    Write-Host "  2) Docker  - containerized (requires Docker Desktop)" -ForegroundColor Cyan
    Write-Host ""
    $choice = Read-Host "  Choose [1/2]"
    $Mode = switch ($choice) {
        "2" { "docker" }
        default { "native" }
    }
}
Write-Ok "Installation mode: $Mode"

# ══════════════════════════════════════════════════════════════════════════════
# Step 2: Check & install dependencies
# ══════════════════════════════════════════════════════════════════════════════
Write-Step "Step 2/6: Checking dependencies"

if ($Mode -eq "docker") {
    # ── Docker ────────────────────────────────────────────────────────────
    if (Test-CommandExists "docker") {
        $dockerVer = (docker --version 2>$null) -replace "Docker version ", "" -replace ",.*", ""
        Write-Ok "Docker $dockerVer found"
    } else {
        Write-Err "Docker not found."
        Write-Info "Please install Docker Desktop for Windows:"
        Write-Info "  https://docs.docker.com/desktop/install/windows-install/"
        exit 1
    }

    # Check Docker daemon
    try {
        docker info 2>$null | Out-Null
        Write-Ok "Docker daemon is running"
    } catch {
        Write-Err "Docker daemon is not running. Please start Docker Desktop."
        exit 1
    }

    # Docker Compose (built into Docker Desktop)
    try {
        $composeVer = docker compose version --short 2>$null
        Write-Ok "Docker Compose $composeVer found"
    } catch {
        Write-Warn "Docker Compose not available — ensure Docker Desktop is up to date."
    }
}

if ($Mode -eq "native") {
    # ── Rust ──────────────────────────────────────────────────────────────
    $missingDeps = @()

    if (Test-CommandExists "cargo") {
        $rustVer = (rustc --version 2>$null) -replace "rustc ", ""
        Write-Ok "Rust $rustVer found"
    } else {
        Write-Warn "Rust not found"
        $missingDeps += "rust"
    }

    # ── Node.js ───────────────────────────────────────────────────────────
    if (Test-CommandExists "node") {
        $nodeVer = node --version 2>$null
        Write-Ok "Node.js $nodeVer found"
    } else {
        Write-Warn "Node.js not found"
        $missingDeps += "node"
    }

    if (Test-CommandExists "npm") {
        $npmVer = npm --version 2>$null
        Write-Ok "npm $npmVer found"
    } elseif ($missingDeps -notcontains "node") {
        Write-Warn "npm not found"
        $missingDeps += "npm"
    }

    # ── Java (optional) ──────────────────────────────────────────────────
    if (Test-CommandExists "javac") {
        $javaVer = (javac -version 2>&1) -replace "javac ", ""
        Write-Ok "Java JDK $javaVer found"
    } else {
        Write-Warn "Java JDK not found (optional - needed for Android builds)"
        $missingDeps += "java"
    }

    # ── FFmpeg (required) ────────────────────────────────────────────────
    if (Test-CommandExists "ffmpeg") {
        $ffmpegVer = (ffmpeg -version 2>$null | Select-Object -First 1) -replace "^ffmpeg version ", ""
        Write-Ok "FFmpeg $ffmpegVer found"
    } else {
        Write-Warn "FFmpeg not found (required for video thumbnails and video/audio edit downloads)"
        $missingDeps += "ffmpeg"
    }

    if ($missingDeps.Count -gt 0) {
        Write-Info "Missing: $($missingDeps -join ', ')"

        if (Read-YesNo "Install missing dependencies?") {
            foreach ($dep in $missingDeps) {
                switch ($dep) {
                    "rust" {
                        Write-Info "Installing Rust..."
                        if (Test-CommandExists "winget") {
                            winget install --id Rustlang.Rustup --accept-source-agreements --accept-package-agreements --silent
                            # Refresh PATH
                            $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
                        } else {
                            Write-Info "Downloading rustup-init.exe..."
                            $rustupUrl = "https://win.rustup.rs/x86_64"
                            $rustupExe = Join-Path $env:TEMP "rustup-init.exe"
                            Invoke-WebRequest -Uri $rustupUrl -OutFile $rustupExe -UseBasicParsing
                            & $rustupExe -y --default-toolchain stable 2>&1 | Select-Object -Last 3
                            $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
                        }
                        if (Test-CommandExists "cargo") {
                            Write-Ok "Rust installed"
                        } else {
                            Write-Err "Rust installation failed. Please install manually: https://rustup.rs"
                            Write-Warn "You may need to restart your terminal after installing."
                            exit 1
                        }
                    }
                    { $_ -in "node", "npm" } {
                        Write-Info "Installing Node.js..."
                        if (Test-CommandExists "winget") {
                            winget install --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements --silent
                        } else {
                            Write-Info "Downloading Node.js installer..."
                            $nodeUrl = "https://nodejs.org/dist/v20.11.0/node-v20.11.0-x64.msi"
                            $nodeMsi = Join-Path $env:TEMP "node-installer.msi"
                            Invoke-WebRequest -Uri $nodeUrl -OutFile $nodeMsi -UseBasicParsing
                            Start-Process msiexec.exe -ArgumentList "/i `"$nodeMsi`" /passive" -Wait
                        }
                        # Refresh PATH for this session
                        $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("PATH", "User")
                        if (Test-CommandExists "node") {
                            Write-Ok "Node.js installed"
                        } else {
                            Write-Err "Node.js installation failed. Please install manually: https://nodejs.org"
                            Write-Warn "You may need to restart your terminal after installing."
                            exit 1
                        }
                    }
                    "java" {
                        Write-Info "Installing Java JDK 17..."
                        if (Test-CommandExists "winget") {
                            winget install --id Microsoft.OpenJDK.17 --accept-source-agreements --accept-package-agreements --silent 2>$null
                        } else {
                            Write-Warn "Cannot auto-install Java. Download from: https://adoptium.net"
                        }
                    }
                    "ffmpeg" {
                        Write-Info "Installing FFmpeg..."
                        $installed = $false
                        if (Test-CommandExists "winget") {
                            winget install --id Gyan.FFmpeg --accept-source-agreements --accept-package-agreements --silent 2>$null
                            # Refresh PATH
                            $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("PATH", "User")
                            $installed = Test-CommandExists "ffmpeg"
                        }
                        if (-not $installed -and (Test-CommandExists "choco")) {
                            choco install ffmpeg -y 2>$null
                            $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("PATH", "User")
                            $installed = Test-CommandExists "ffmpeg"
                        }
                        if ($installed) {
                            Write-Ok "FFmpeg installed"
                        } else {
                            Write-Err "Could not auto-install FFmpeg. Download from: https://ffmpeg.org/download.html"
                            Write-Err "FFmpeg is required for video thumbnails and baking edits into video/audio downloads."
                            exit 1
                        }
                    }
                }
            }
        } else {
            if ($missingDeps -contains "rust" -or $missingDeps -contains "node" -or $missingDeps -contains "ffmpeg") {
                Write-Err "Rust, Node.js, and FFmpeg are required for native mode."
                exit 1
            }
        }
    }
}

# ══════════════════════════════════════════════════════════════════════════════
# Step 4: Port configuration
# ══════════════════════════════════════════════════════════════════════════════
Write-Step "Step 3/6: Port configuration"

if ($Port -eq 0) {
    $Port = [int](Read-Prompt "Server port" $DefaultPort)
}

$FinalPort = Find-AvailablePort $Port
if ($FinalPort -ne $Port) {
    Write-Info "Port $Port was busy -> using port $FinalPort"
}
$Port = $FinalPort
Write-Ok "Server will run on port $Port"

# ══════════════════════════════════════════════════════════════════════════════
# Step 5: Configuration
# ══════════════════════════════════════════════════════════════════════════════
Write-Step "Step 4/6: Configuration"

if (-not $Name) {
    $defaultName = if ($Mode -eq "docker") { "simple-photos-$Port" } else { "simple-photos" }
    $Name = Read-Prompt "Instance name" $defaultName
}
Write-Ok "Instance: $Name"

if (-not $StoragePath) {
    $defaultStorage = Join-Path $ScriptDir "server\data\storage"
    $StoragePath = Read-Prompt "Photo storage path" $defaultStorage
}
if (-not (Test-Path $StoragePath)) {
    New-Item -ItemType Directory -Path $StoragePath -Force | Out-Null
}
Write-Ok "Storage: $StoragePath"

$JwtSecret = New-SecureKey

# ══════════════════════════════════════════════════════════════════════════════
# Step 5: Build & Install
# ══════════════════════════════════════════════════════════════════════════════
Write-Step "Step 5/6: Building"

function Write-Config {
    param(
        [string]$Dest,
        [int]$CfgPort,
        [string]$CfgStorage,
        [string]$CfgDb,
        [string]$CfgWeb,
        [string]$CfgBaseUrl
    )

    $backupLine = ""

    # Escape backslashes for TOML
    $CfgStorage = $CfgStorage -replace "\\", "\\"
    $CfgDb = $CfgDb -replace "\\", "\\"
    $CfgWeb = $CfgWeb -replace "\\", "\\"

    $config = @"
[server]
host = "0.0.0.0"
port = $CfgPort
base_url = "$CfgBaseUrl"

[database]
path = "$CfgDb"
max_connections = 5

[storage]
root = "$CfgStorage"
default_quota_bytes = 10737418240
max_blob_size_bytes = 5368709120

[auth]
jwt_secret = "$JwtSecret"
access_token_ttl_secs = 3600
refresh_token_ttl_days = 30
allow_registration = true
bcrypt_cost = 12

[web]
static_root = "$CfgWeb"

[backup]
$backupLine

[tls]
enabled = false
"@
    Set-Content -Path $Dest -Value $config -Encoding UTF8
}

# Append a [tls.letsencrypt] stanza when the operator supplied LE flags.
# Mirrors maybe_write_letsencrypt_stanza in install.sh — the running
# server performs the actual ACME flow once the wizard's SSL step is
# completed (or admin clicks "Issue certificate" in Settings → SSL/TLS).
function Add-LetsEncryptStanza {
    param([string]$Dest)
    if (-not $LetsEncryptDomain -or -not $LetsEncryptEmail) { return }
    if (-not $LetsEncryptAgreeTos) {
        Write-Warn "Let's Encrypt flags supplied without -LetsEncryptAgreeTos -- skipping config stub."
        Write-Warn "Re-run with -LetsEncryptAgreeTos to accept https://letsencrypt.org/repository/."
        return
    }
    $stagingValue = if ($LetsEncryptStaging) { "true" } else { "false" }
    $stanza = @"

[tls.letsencrypt]
domain = "$LetsEncryptDomain"
email = "$LetsEncryptEmail"
staging = $stagingValue
challenge_port = 80
"@
    Add-Content -Path $Dest -Value $stanza -Encoding UTF8
    Write-Ok "Pre-seeded [tls.letsencrypt] for $LetsEncryptDomain (complete in setup wizard)."
}

if ($Mode -eq "native") {
    # ── Build web frontend ────────────────────────────────────────────────
    Write-Info "Installing npm packages..."
    Set-Location (Join-Path $ScriptDir "web")
    npm install --silent 2>&1 | Select-Object -Last 1
    Write-Info "Building React app..."
    npm run build 2>&1 | Select-Object -Last 3
    Write-Ok "Web frontend built -> web\dist\"
    Set-Location $ScriptDir

    # ── Build Rust server ─────────────────────────────────────────────────
    Write-Info "Building server (release)... may take a few minutes on first run."
    Set-Location (Join-Path $ScriptDir "server")

    # CUDA execution provider is baked into the default build (see
    # server\Cargo.toml [features]). The ORT CUDA EP loads cudart lazily
    # at runtime, so the same binary runs on GPU and CPU hosts.
    Write-Info "Building with CUDA execution provider baked in (auto-falls back to CPU at runtime)"
    cargo build --release 2>&1 | Select-Object -Last 5
    Write-Ok "Server built -> server\target\release\simple-photos-server.exe"
    Set-Location $ScriptDir

    # ── AI models + GeoNames dataset ──────────────────────────────────────
    if ($SkipModels) {
        Write-Warn "Skipping AI models / GeoNames dataset download (-SkipModels). Server will start in degraded_mode until models are downloaded."
    } else {
        Write-Info "Fetching AI ONNX models -> server\models\  (~200 MB, mandatory for face/object recognition)"
        try {
            Download-AiModels -Target (Join-Path $ScriptDir "server\models")
            Write-Ok "AI models installed"
        } catch {
            Write-Err "AI model download failed: $_"
            Write-Err "Re-run install.ps1 or call Download-AiModels manually before starting the server, or pass -SkipModels to install without AI features."
            exit 1
        }

        Write-Info "Fetching GeoNames cities500 -> server\data\cities500.txt  (~25 MB, mandatory for reverse geocoding)"
        try {
            Download-GeoData -Target (Join-Path $ScriptDir "server\data\cities500.txt")
            Write-Ok "GeoNames dataset installed"
        } catch {
            Write-Warn "Geo dataset download failed: $_"
            Write-Warn "Reverse-geocoding will be disabled until you re-run install.ps1 or call Download-GeoData manually."
        }
    }

    # ── Config ────────────────────────────────────────────────────────────
    $configPath = Join-Path $ScriptDir "server\config.toml"
    Write-Config `
        -Dest $configPath `
        -CfgPort $Port `
        -CfgStorage $StoragePath `
        -CfgDb ".\data\db\simple-photos.db" `
        -CfgWeb "..\web\dist" `
        -CfgBaseUrl "http://localhost:$Port"
    Add-LetsEncryptStanza -Dest $configPath
    Write-Ok "Config -> server\config.toml"

    # ── Data directories ──────────────────────────────────────────────────
    $dbDir = Join-Path $ScriptDir "server\data\db"
    $dlDir = Join-Path $ScriptDir "downloads"
    @($dbDir, $StoragePath, $dlDir) | ForEach-Object {
        if (-not (Test-Path $_)) { New-Item -ItemType Directory -Path $_ -Force | Out-Null }
    }
    Write-Ok "Data directories ready"

    # ── Windows Task Scheduler for auto-start ─────────────────────────────
    $serverExe = Join-Path $ScriptDir "server\target\release\simple-photos-server.exe"

    if (Read-YesNo "Register as a Windows scheduled task (auto-start on login)?" "Y") {
        try {
            $taskName = "SimplePhotosServer"
            $serverDir = Join-Path $ScriptDir "server"

            # Remove existing task if present
            Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue

            $action  = New-ScheduledTaskAction `
                -Execute $serverExe `
                -WorkingDirectory $serverDir
            $trigger = New-ScheduledTaskTrigger -AtLogon
            $settings = New-ScheduledTaskSettingsSet `
                -AllowStartIfOnBatteries `
                -DontStopIfGoingOnBatteries `
                -StartWhenAvailable `
                -RestartCount 3 `
                -RestartInterval (New-TimeSpan -Minutes 1)

            Register-ScheduledTask `
                -TaskName $taskName `
                -Action $action `
                -Trigger $trigger `
                -Settings $settings `
                -Description "Simple Photos Server - self-hosted E2E encrypted photo library" `
                -RunLevel Highest | Out-Null

            Write-Ok "Scheduled task '$taskName' registered (auto-starts on login)"
            Write-Info "Manage via: Task Scheduler -> $taskName"
        } catch {
            Write-Warn "Could not register scheduled task (may need admin rights)."
            Write-Warn "You can start the server manually: .\server\target\release\simple-photos-server.exe"
        }
    }

    # ── Windows Firewall rule ─────────────────────────────────────────────
    if (Read-YesNo "Add Windows Firewall rule to allow port $Port?" "Y") {
        try {
            $ruleName = "SimplePhotos-Port-$Port"
            # Remove existing rule if present
            Remove-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue
            New-NetFirewallRule `
                -DisplayName $ruleName `
                -Direction Inbound `
                -Protocol TCP `
                -LocalPort $Port `
                -Action Allow `
                -Profile Private, Domain | Out-Null
            Write-Ok "Firewall rule added: $ruleName (TCP $Port, Private+Domain networks)"
        } catch {
            Write-Warn "Could not add firewall rule (may need admin rights)."
            Write-Warn "Run as Administrator, or manually allow TCP port $Port in Windows Firewall."
        }
    }

} elseif ($Mode -eq "docker") {
    # ── Build web frontend if needed ──────────────────────────────────────
    $webDist = Join-Path $ScriptDir "web\dist"
    if (-not (Test-Path $webDist)) {
        if (Test-CommandExists "npm") {
            Write-Info "Building web frontend..."
            Set-Location (Join-Path $ScriptDir "web")
            npm install --silent 2>&1 | Select-Object -Last 1
            npm run build 2>&1 | Select-Object -Last 3
            Set-Location $ScriptDir
            Write-Ok "Web frontend built"
        } else {
            Write-Warn "npm not available - ensure web\dist exists before starting."
        }
    } else {
        Write-Ok "Web frontend already built -> web\dist\"
    }

    # ── Instance directory ────────────────────────────────────────────────
    $instanceDir = Join-Path $ScriptDir "docker-instances\$Name"
    $instanceDbDir = Join-Path $instanceDir "data\db"
    $instanceStorageDir = Join-Path $instanceDir "data\storage"
    @($instanceDbDir, $instanceStorageDir) | ForEach-Object {
        if (-not (Test-Path $_)) { New-Item -ItemType Directory -Path $_ -Force | Out-Null }
    }

    # Inside container, internal port is always 3000
    $dockerConfigPath = Join-Path $instanceDir "config.toml"
    Write-Config `
        -Dest $dockerConfigPath `
        -CfgPort 3000 `
        -CfgStorage "/data/storage" `
        -CfgDb "/data/db/simple-photos.db" `
        -CfgWeb "/app/web/dist" `
        -CfgBaseUrl "http://localhost:$Port"
    Add-LetsEncryptStanza -Dest $dockerConfigPath
    Write-Ok "Config -> docker-instances\$Name\config.toml"

    # ── docker-compose.yml ────────────────────────────────────────────────
    # Convert Windows paths to Docker-compatible format
    $dockerInstanceDir = $instanceDir -replace "\\", "/"
    $dockerScriptDir = $ScriptDir -replace "\\", "/"
    $dockerStoragePath = $StoragePath -replace "\\", "/"

    $composeContent = @"
services:
  server:
    build:
      context: $dockerScriptDir/server
      dockerfile: Dockerfile
    container_name: $Name
    restart: unless-stopped
    ports:
      - "${Port}:3000"
    volumes:
      - $dockerInstanceDir/config.toml:/app/config.toml:ro
      - $dockerScriptDir/web/dist:/app/web/dist:ro
      - $dockerInstanceDir/data/db:/data/db
      - $dockerStoragePath`:/data/storage
    environment:
      - RUST_LOG=info
    networks:
      - simple-photos-net
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:3000/api/health"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 10s

networks:
  simple-photos-net:
    name: simple-photos-net
    external: true
"@
    Set-Content -Path (Join-Path $instanceDir "docker-compose.yml") -Value $composeContent -Encoding UTF8
    Write-Ok "Docker Compose -> docker-instances\$Name\docker-compose.yml"

    # ── Shared network ────────────────────────────────────────────────────
    $networks = docker network ls --format "{{.Name}}" 2>$null
    if ($networks -notcontains "simple-photos-net") {
        docker network create simple-photos-net 2>$null | Out-Null
        Write-Ok "Created Docker network: simple-photos-net"
    } else {
        Write-Ok "Docker network simple-photos-net exists"
    }

    # ── Build image ───────────────────────────────────────────────────────
    Write-Info "Building Docker image... (may take a few minutes on first run)"
    Set-Location $instanceDir
    docker compose build 2>&1 | Select-Object -Last 10
    Write-Ok "Docker image built for $Name"
    Set-Location $ScriptDir
}

# ══════════════════════════════════════════════════════════════════════════════
# Step 6.5: Android APK (optional, native only)
# ══════════════════════════════════════════════════════════════════════════════
if ($Mode -eq "native" -and -not $NoBuildAndroid) {
    $androidDir = Join-Path $ScriptDir "android"
    if ((Test-CommandExists "javac") -and (Test-Path $androidDir)) {
        Write-Step "Step 6/7: Android app (optional)"
        if (Read-YesNo "Build the Android APK?" "N") {
            Write-Info "Building Android APK..."
            Set-Location $androidDir
            $gradlew = Join-Path $androidDir "gradlew.bat"
            if (Test-Path $gradlew) {
                try {
                    & $gradlew assembleDebug 2>&1 | Select-Object -Last 5
                    Write-Ok "APK built"
                } catch {
                    Write-Warn "APK build failed: $_"
                }
            } else {
                Write-Warn "gradlew.bat not found in android directory"
            }
            Set-Location $ScriptDir
        } else {
            Write-Info "Skipping Android build."
        }
    }
}

# ══════════════════════════════════════════════════════════════════════════════
# Step 7: Summary & Launch
# ══════════════════════════════════════════════════════════════════════════════
Write-Step "Step 7/7: Ready!"

Write-Host ""
Write-Host "  Simple Photos is installed and ready!" -ForegroundColor Green
Write-Host ""
Write-Host "  Mode:     $Mode"
Write-Host "  Port:     $Port"
Write-Host "  Name:     $Name"
Write-Host "  Storage:  $StoragePath"
Write-Host ""

if ($Mode -eq "native") {
    $serverExe = Join-Path $ScriptDir "server\target\release\simple-photos-server.exe"
    Write-Host "  Start:    .\server\target\release\simple-photos-server.exe"
    Write-Host "  Or:       schtasks /run /tn SimplePhotosServer"
    Write-Host "  Stop:     schtasks /end /tn SimplePhotosServer"
    Write-Host "  Open:     http://localhost:$Port" -ForegroundColor Cyan
} else {
    Write-Host "  Start:    cd docker-instances\$Name; docker compose up -d"
    Write-Host "  Stop:     cd docker-instances\$Name; docker compose down"
    Write-Host "  Logs:     cd docker-instances\$Name; docker compose logs -f"
    Write-Host "  Open:     http://localhost:$Port" -ForegroundColor Cyan
}
Write-Host ""

if (-not $NoStart) {
    if (Read-YesNo "Start the server now?") {
        Write-Host ""
        if ($Mode -eq "native") {
            Write-Info "Starting on port $Port..."
            Write-Host "  -> http://localhost:$Port" -ForegroundColor Cyan
            Write-Host "  Press Ctrl+C to stop.`n"
            Set-Location (Join-Path $ScriptDir "server")
            & $serverExe
        } else {
            Write-Info "Starting container: $Name"
            Set-Location (Join-Path $ScriptDir "docker-instances\$Name")
            docker compose up -d
            Write-Host "`n  -> http://localhost:$Port`n" -ForegroundColor Cyan
            Write-Ok "Container $Name is running"
            Set-Location $ScriptDir
        }
    }
}

# ── Local-CA hint ─────────────────────────────────────────────────────
# Mirror the install.sh post-install hint so Windows operators who pass
# -LocalCa get the same instructions for generating the self-signed CA
# and downloading the install bundle from the web UI.
if ($LocalCa) {
    Write-Host ""
    Write-Info 'Self-signed local CA was requested (-LocalCa).'
    Write-Info 'Open the web UI -> Settings -> SSL / TLS -> "Self-signed local CA"'
    Write-Info 'and click "Generate local CA".  Then click "Download CA install'
    Write-Info 'bundle" and run the included script on each device:'
    Write-Info '  Linux:    sudo ./install-linux.sh'
    Write-Info '  Windows:  PowerShell (as admin) -> .\install-windows.ps1'
    Write-Info '  Android:  follow install-android.txt'
    Write-Info 'After installing the CA on a device, the Simple Photos URL will'
    Write-Info 'load as a fully-trusted HTTPS site with no browser warnings.'
}
