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

.PARAMETER Role
    Server role: "primary" or "backup". Default: primary

.PARAMETER Name
    Instance name (for Docker containers). Default: auto-generated

.PARAMETER StoragePath
    Path to photo storage directory

.PARAMETER AdminUser
    Admin username (skip interactive prompt)

.PARAMETER AdminPass
    Admin password (skip interactive prompt)

.PARAMETER BackupApiKey
    Backup API key for backup servers

.PARAMETER PrimaryUrl
    Primary server URL (for backup pairing)

.PARAMETER NoBuildAndroid
    Skip Android APK build prompt

.PARAMETER NoStart
    Don't start the server after install

.PARAMETER Yes
    Auto-accept all prompts

.EXAMPLE
    .\install.ps1
    .\install.ps1 -Mode native -Port 8080
    .\install.ps1 -Mode docker -Port 8080
    .\install.ps1 -Mode docker -Role backup -Port 8081
#>

[CmdletBinding()]
param(
    [ValidateSet("native", "docker", "")]
    [string]$Mode = "",

    [int]$Port = 0,

    [ValidateSet("primary", "backup", "")]
    [string]$Role = "",

    [string]$Name = "",
    [string]$StoragePath = "",
    [string]$AdminUser = "",
    [string]$AdminPass = "",
    [string]$BackupApiKey = "",
    [string]$PrimaryUrl = "",
    [switch]$NoBuildAndroid,
    [switch]$NoStart,
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
# Step 2: Server role
# ══════════════════════════════════════════════════════════════════════════════
if (-not $Role) {
    if (-not $Yes) {
        Write-Host ""
        Write-Host "  Server role:" -ForegroundColor White
        Write-Host "  1) Primary - main server for uploading & managing photos" -ForegroundColor Cyan
        Write-Host "  2) Backup  - backup server that syncs from a primary" -ForegroundColor Cyan
        Write-Host ""
        $choice = Read-Host "  Choose [1/2]"
        $Role = switch ($choice) {
            "2" { "backup" }
            default { "primary" }
        }
    } else {
        $Role = "primary"
    }
}
Write-Ok "Server role: $Role"

# ══════════════════════════════════════════════════════════════════════════════
# Step 3: Check & install dependencies
# ══════════════════════════════════════════════════════════════════════════════
Write-Step "Step 2/7: Checking dependencies"

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
Write-Step "Step 3/7: Port configuration"

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
Write-Step "Step 4/7: Configuration"

if (-not $Name) {
    $defaultName = if ($Mode -eq "docker") { "simple-photos-$Role-$Port" } else { "simple-photos" }
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

if ($Role -eq "backup" -and -not $BackupApiKey) {
    $BackupApiKey = New-SecureKey
    Write-Info "Generated backup API key: $($BackupApiKey.Substring(0, 16))..."
}

if ($Role -eq "backup" -and -not $PrimaryUrl -and -not $Yes) {
    $PrimaryUrl = Read-Prompt "Primary server URL (e.g., http://localhost:8080)" ""
}

# ══════════════════════════════════════════════════════════════════════════════
# Step 6: Build & Install
# ══════════════════════════════════════════════════════════════════════════════
Write-Step "Step 5/7: Building"

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
    if ($BackupApiKey) {
        $backupLine = "api_key = `"$BackupApiKey`""
    }

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
    cargo build --release 2>&1 | Select-Object -Last 5
    Write-Ok "Server built -> server\target\release\simple-photos-server.exe"
    Set-Location $ScriptDir

    # ── Config ────────────────────────────────────────────────────────────
    $configPath = Join-Path $ScriptDir "server\config.toml"
    Write-Config `
        -Dest $configPath `
        -CfgPort $Port `
        -CfgStorage $StoragePath `
        -CfgDb ".\data\db\simple-photos.db" `
        -CfgWeb "..\web\dist" `
        -CfgBaseUrl "http://localhost:$Port"
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
    Write-Config `
        -Dest (Join-Path $instanceDir "config.toml") `
        -CfgPort 3000 `
        -CfgStorage "/data/storage" `
        -CfgDb "/data/db/simple-photos.db" `
        -CfgWeb "/app/web/dist" `
        -CfgBaseUrl "http://localhost:$Port"
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
Write-Host "  Role:     $Role"
Write-Host "  Port:     $Port"
Write-Host "  Name:     $Name"
Write-Host "  Storage:  $StoragePath"
Write-Host ""

if ($BackupApiKey) {
    Write-Host "  Backup API Key: $BackupApiKey" -ForegroundColor White
    Write-Host "  (Save this - needed to register this server as a backup target)" -ForegroundColor Yellow
    Write-Host ""
}

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
