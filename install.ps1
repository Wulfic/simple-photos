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

.PARAMETER Uninstall
    Remove an existing installation: "native" or "docker"

.NOTES
    The photo storage location is configured by the first-run setup wizard
    (web UI). The installer scaffolds a default directory only.

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
#>

[CmdletBinding()]
param(
    [ValidateSet("native", "docker", "")]
    [string]$Mode = "",

    [int]$Port = 0,

    [string]$Name = "",
    [string]$Uninstall = "",
    [string]$LetsEncryptDomain = "",
    [string]$LetsEncryptEmail = "",
    [switch]$LetsEncryptStaging,
    [switch]$LetsEncryptAgreeTos,
    [switch]$LocalCa,
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

# Download a single URL to $OutPath, skipping if the file already exists.
function Invoke-FileDownload {
    param([string]$Url, [string]$OutPath)
    if ((Test-Path $OutPath) -and (Get-Item $OutPath).Length -gt 0) {
        Write-Info "[dl] $(Split-Path $OutPath -Leaf) already present — skipping"
        return
    }
    Write-Info "[dl] Downloading $(Split-Path $OutPath -Leaf)…"
    $part = "$OutPath.$([System.IO.Path]::GetRandomFileName()).part"
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

    Write-Info "[ai] Models present in ${Target}:"
    Get-ChildItem $Target | ForEach-Object { Write-Info "  $($_.Name)  ($([math]::Round($_.Length/1MB,1)) MB)" }
}

# Install the Android SDK command-line tools for Windows and set up the SDK
# components required to build the Android APK (compileSdk=34, buildTools=34.0.0).
function Install-AndroidSdk {
    param([string]$SdkRoot = (Join-Path $env:USERPROFILE "android-sdk"))

    Write-Info "Installing Android SDK command-line tools to $SdkRoot..."

    $cmdlineUrl = "https://dl.google.com/android/repository/commandlinetools-win-11076708_latest.zip"
    $zipPath = Join-Path $env:TEMP "android-cmdtools-win.zip"
    try {
        Invoke-FileDownload -Url $cmdlineUrl -OutPath $zipPath

        $extractTarget = Join-Path $SdkRoot "cmdline-tools"
        if (-not (Test-Path $extractTarget)) { New-Item -ItemType Directory -Path $extractTarget -Force | Out-Null }

        Write-Info "Extracting Android command-line tools..."
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        [System.IO.Compression.ZipFile]::ExtractToDirectory($zipPath, $extractTarget)

        # The zip extracts to cmdline-tools/cmdline-tools — rename to latest
        $innerDir = Join-Path $extractTarget "cmdline-tools"
        $latestDir = Join-Path $extractTarget "latest"
        if ((Test-Path $innerDir) -and -not (Test-Path $latestDir)) {
            Rename-Item -Path $innerDir -NewName "latest"
        }
    } finally {
        Remove-Item $zipPath -ErrorAction SilentlyContinue
    }

    $env:ANDROID_HOME = $SdkRoot
    $env:PATH = "$SdkRoot\cmdline-tools\latest\bin;$SdkRoot\platform-tools;$env:PATH"

    # Accept all SDK licenses non-interactively
    $sdkmanager = Join-Path $SdkRoot "cmdline-tools\latest\bin\sdkmanager.bat"
    if (Test-Path $sdkmanager) {
        Write-Info "Accepting Android SDK licenses..."
        "y`ny`ny`ny`ny`ny`n" | & $sdkmanager --licenses 2>$null | Out-Null

        Write-Info "Installing Android SDK components (platform-tools, android-34, build-tools 34.0.0)..."
        & $sdkmanager "platform-tools" "platforms;android-34" "build-tools;34.0.0" 2>&1 | Select-Object -Last 5

        # Persist ANDROID_HOME to user environment
        [System.Environment]::SetEnvironmentVariable("ANDROID_HOME", $SdkRoot, "User")
        $userPath = [System.Environment]::GetEnvironmentVariable("PATH", "User")
        $addPaths = @("$SdkRoot\cmdline-tools\latest\bin", "$SdkRoot\platform-tools")
        foreach ($p in $addPaths) {
            if ($userPath -notlike "*$p*") { $userPath = "$p;$userPath" }
        }
        [System.Environment]::SetEnvironmentVariable("PATH", $userPath, "User")

        Write-Ok "Android SDK installed at $SdkRoot"
    } else {
        throw "sdkmanager.bat not found after extraction — Android SDK install failed."
    }
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

        # Companion file: admin1 region names ("California" instead of the
        # raw GeoNames code "CA" / "07").  The server looks for it next to
        # cities500.txt.  install.sh already fetched this; without it every
        # non-US state renders as a meaningless numeric code.
        $adminTarget = Join-Path $dir "admin1CodesASCII.txt"
        try {
            Write-Info "[geo] Downloading GeoNames admin1 region names…"
            Invoke-WebRequest `
                -Uri "https://download.geonames.org/export/dump/admin1CodesASCII.txt" `
                -OutFile $adminTarget -UseBasicParsing
            Write-Info "[geo] Done — region names written to $adminTarget"
        } catch {
            Write-Warn "[geo] admin1CodesASCII.txt download failed ($($_.Exception.Message)) — states will show as raw codes"
        }
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
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    return ($bytes | ForEach-Object { $_.ToString("x2") }) -join ""
}

function Test-CommandExists {
    param([string]$Cmd)
    return [bool](Get-Command $Cmd -ErrorAction SilentlyContinue)
}

# Run a native build command (npm, vite, cargo, gradlew) tolerating stderr.
# The script sets `$ErrorActionPreference = "Stop"` globally for cmdlet
# safety, but that also turns *any* native-command stderr line into a
# terminating NativeCommandError. npm, vite, and cargo all write ordinary
# progress and warnings to stderr even on success, which would otherwise
# abort the install. We relax the preference for the duration of the call
# and gate success on the real process exit code ($LASTEXITCODE) instead.
function Invoke-NativeBuild {
    param(
        [Parameter(Mandatory)] [scriptblock]$Command,
        [string]$What = "build step",
        [int]$Tail = 5
    )
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Command 2>&1 | Select-Object -Last $Tail
    } finally {
        $ErrorActionPreference = $prev
    }
    if ($LASTEXITCODE -ne 0) {
        throw "$What failed (exit code $LASTEXITCODE)."
    }
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
# Uninstall mode
# ══════════════════════════════════════════════════════════════════════════════
if ($Uninstall) {
    switch ($Uninstall.ToLower()) {
        "native" {
            Write-Step "Uninstalling Simple Photos (native)"
            $taskName = "SimplePhotosServer"
            if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) {
                Write-Info "Stopping and removing scheduled task: $taskName"
                Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
                Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
                Write-Ok "Scheduled task removed"
            }
            Get-NetFirewallRule -DisplayName "SimplePhotos-Port-*" -ErrorAction SilentlyContinue | ForEach-Object {
                Remove-NetFirewallRule -Name $_.Name -ErrorAction SilentlyContinue
                Write-Ok "Firewall rule removed: $($_.DisplayName)"
            }
            $serverExe = Join-Path $ScriptDir "server\target\release\simple-photos-server.exe"
            $configPath = Join-Path $ScriptDir "server\config.toml"
            if ((Test-Path $serverExe) -or (Test-Path $configPath)) {
                if (Read-YesNo "Remove built server binary and config.toml?" "Y") {
                    Remove-Item $serverExe -ErrorAction SilentlyContinue
                    Remove-Item $configPath -ErrorAction SilentlyContinue
                    Write-Ok "Server binary and config removed"
                }
            }
            Write-Host ""
            Write-Ok "Native uninstall complete."
            Write-Warn "Photo storage data was NOT removed: $(Join-Path $ScriptDir 'server\data\storage')"
            Write-Info "Remove it manually if no longer needed."
            exit 0
        }
        "docker" {
            Write-Step "Uninstalling Simple Photos (docker)"
            if (-not (Test-CommandExists "docker")) {
                Write-Err "Docker not found. Cannot perform Docker uninstall."
                exit 1
            }
            $instancesDir = Join-Path $ScriptDir "docker-instances"
            if (-not (Test-Path $instancesDir)) {
                Write-Warn "No docker-instances directory found."
                exit 0
            }
            $targets = if ($Name) {
                @(Join-Path $instancesDir $Name)
            } else {
                Get-ChildItem $instancesDir -Directory | Select-Object -ExpandProperty FullName
            }
            foreach ($instDir in $targets) {
                if (-not (Test-Path $instDir)) { continue }
                $instName = Split-Path $instDir -Leaf
                $composePath = Join-Path $instDir "docker-compose.yml"
                if (Test-Path $composePath) {
                    Write-Info "Stopping container: $instName"
                    Push-Location $instDir
                    docker compose down 2>$null
                    Pop-Location
                }
                if (Read-YesNo "Remove instance directory: docker-instances\$instName?" "Y") {
                    Remove-Item $instDir -Recurse -Force
                    Write-Ok "Removed: docker-instances\$instName"
                }
            }
            Write-Host ""
            Write-Ok "Docker uninstall complete."
            Write-Warn "Photo storage data outside the default scaffold path was NOT removed; remove any custom storage roots configured in the setup wizard manually."
            Write-Info "Remove it manually if no longer needed."
            exit 0
        }
        default {
            Write-Err "Unknown uninstall mode: '$Uninstall' (use 'native' or 'docker')"
            exit 1
        }
    }
}

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

    # ── Android SDK (optional, needed for APK build) ──────────────────────
    if (-not $NoBuildAndroid) {
        $androidSdkOk = $false
        $sdkManagerBat = ""
        if ($env:ANDROID_HOME -and (Test-Path (Join-Path $env:ANDROID_HOME "cmdline-tools\latest\bin\sdkmanager.bat"))) {
            $sdkManagerBat = Join-Path $env:ANDROID_HOME "cmdline-tools\latest\bin\sdkmanager.bat"
            $env:PATH = "$env:ANDROID_HOME\cmdline-tools\latest\bin;$env:ANDROID_HOME\platform-tools;$env:PATH"
            $androidSdkOk = $true
        } elseif (Test-CommandExists "sdkmanager") {
            $androidSdkOk = $true
        } elseif (Test-Path (Join-Path $env:USERPROFILE "android-sdk\cmdline-tools\latest\bin\sdkmanager.bat")) {
            $env:ANDROID_HOME = Join-Path $env:USERPROFILE "android-sdk"
            $env:PATH = "$env:ANDROID_HOME\cmdline-tools\latest\bin;$env:ANDROID_HOME\platform-tools;$env:PATH"
            $androidSdkOk = $true
        }
        if ($androidSdkOk) {
            Write-Ok "Android SDK found ($env:ANDROID_HOME)"
        } else {
            Write-Warn "Android SDK not found (required for Android APK build)"
            $missingDeps += "android-sdk"
        }
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
                            # Refresh PATH
                            $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("PATH", "User")
                            if (Test-CommandExists "javac") {
                                Write-Ok "Java JDK 17 installed"
                            } else {
                                Write-Warn "Java not found on PATH yet — you may need to reopen your terminal."
                            }
                        } else {
                            Write-Warn "Cannot auto-install Java. Download from: https://adoptium.net"
                        }
                    }
                    "android-sdk" {
                        # Java must be present before installing the SDK
                        if (-not (Test-CommandExists "javac")) {
                            Write-Warn "Java JDK is required before installing the Android SDK."
                            Write-Warn "Install Java first (https://adoptium.net) and re-run this script."
                        } else {
                            try {
                                Install-AndroidSdk
                            } catch {
                                Write-Warn "Android SDK install failed: $_"
                                Write-Warn "You can install it manually from https://developer.android.com/studio#command-tools"
                            }
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
    $Port = $DefaultPort
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
    $defaultName = if ($Mode -eq "docker") { "simple-photos-$Port" } else { "simple-photos" }
    $Name = Read-Prompt "Instance name" $defaultName
}
Write-Ok "Instance: $Name"

# Default scaffold path; storage root is finalised in the first-run setup wizard.
if ($Mode -eq "docker") {
    $StoragePath = Join-Path $ScriptDir "docker-instances\$Name\data\storage"
} else {
    $StoragePath = Join-Path $ScriptDir "server\data\storage"
}
if (-not (Test-Path $StoragePath)) {
    New-Item -ItemType Directory -Path $StoragePath -Force | Out-Null
}
Write-Ok "Storage (default scaffold): $StoragePath"

$JwtSecret = New-SecureKey

# ══════════════════════════════════════════════════════════════════════════════
# Step 5: Build & Install
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
    Invoke-NativeBuild -What "npm install" -Tail 1 -Command { npm install --silent }
    Write-Info "Building React app..."
    Invoke-NativeBuild -What "npm run build" -Tail 3 -Command { npm run build }
    Write-Ok "Web frontend built -> web\dist\"
    Set-Location $ScriptDir

    # ── Build Rust server ─────────────────────────────────────────────────
    Write-Info "Building server (release)... may take a few minutes on first run."
    Set-Location (Join-Path $ScriptDir "server")

    # CUDA execution provider is baked into the default build (see
    # server\Cargo.toml [features]). The ORT CUDA EP loads cudart lazily
    # at runtime, so the same binary runs on GPU and CPU hosts.
    Write-Info "Building with CUDA execution provider baked in (auto-falls back to CPU at runtime)"
    Invoke-NativeBuild -What "cargo build --release" -Tail 5 -Command { cargo build --release }
    Write-Ok "Server built -> server\target\release\simple-photos-server.exe"
    Set-Location $ScriptDir

    # ── AI models + GeoNames dataset (mandatory) ─────────────────────────
    # Face/object recognition and reverse geocoding have no working
    # fallbacks, so the installer always downloads these assets.
    Write-Info "Fetching AI ONNX models -> server\models\  (~200 MB, mandatory for face/object recognition)"
    try {
        Download-AiModels -Target (Join-Path $ScriptDir "server\models")
        Write-Ok "AI models installed"
    } catch {
        Write-Err "AI model download failed: $_"
        Write-Err "Re-run install.ps1 once the network issue is resolved."
        exit 1
    }

    Write-Info "Fetching GeoNames cities500 -> server\data\cities500.txt  (~25 MB, mandatory for reverse geocoding)"
    try {
        Download-GeoData -Target (Join-Path $ScriptDir "server\data\cities500.txt")
        Write-Ok "GeoNames dataset installed"
    } catch {
        Write-Warn "Geo dataset download failed: $_"
        Write-Warn "Reverse-geocoding will be disabled until you re-run install.ps1."
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
    # The task runs server\watchdog.ps1 rather than the server exe directly.
    # The watchdog restarts the server if it exits, but waits 10 seconds
    # before doing so.  During that window the user can also kill the watchdog
    # process in Task Manager → Details (powershell.exe running watchdog.ps1),
    # which prevents the restart entirely.  This mirrors the behaviour of the
    # NSSM-managed Windows Service created by the packaged installer.
    $serverExe     = Join-Path $ScriptDir "server\target\release\simple-photos-server.exe"
    $watchdogScript = Join-Path $ScriptDir "server\watchdog.ps1"

    if (Read-YesNo "Register as a Windows scheduled task (auto-start on login)?" "Y") {
        try {
            $taskName  = "SimplePhotosServer"
            $serverDir = Join-Path $ScriptDir "server"

            # Remove existing task if present
            Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue

            # Run the watchdog script via powershell.exe so the supervisor loop
            # and the server process are two distinct entries in Task Manager.
            $action  = New-ScheduledTaskAction `
                -Execute  "powershell.exe" `
                -Argument "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$watchdogScript`"" `
                -WorkingDirectory $serverDir
            $trigger  = New-ScheduledTaskTrigger -AtLogon
            # No RestartCount/RestartInterval here — the watchdog script owns
            # the restart logic with its built-in 10-second grace delay.
            $settings = New-ScheduledTaskSettingsSet `
                -AllowStartIfOnBatteries `
                -DontStopIfGoingOnBatteries `
                -StartWhenAvailable

            Register-ScheduledTask `
                -TaskName    $taskName `
                -Action      $action `
                -Trigger     $trigger `
                -Settings    $settings `
                -Description "Simple Photos Server watchdog — self-hosted E2E encrypted photo library" `
                -RunLevel    Highest | Out-Null

            Write-Ok "Scheduled task '$taskName' registered (auto-starts on login via watchdog)"
            Write-Info "Manage via: Task Scheduler -> $taskName"
            Write-Info "To temporarily stop: kill the powershell.exe watchdog in Task Manager -> Details"
            Write-Info "  (after killing the watchdog you have 10 s to also kill simple-photos-server.exe)"
            Write-Info "To permanently stop: schtasks /End /TN $taskName  then  schtasks /Change /TN $taskName /DISABLE"
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
            Invoke-NativeBuild -What "npm install" -Tail 1 -Command { npm install --silent }
            Invoke-NativeBuild -What "npm run build" -Tail 3 -Command { npm run build }
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
    Invoke-NativeBuild -What "docker compose build" -Tail 10 -Command { docker compose build }
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

        # Resolve ANDROID_HOME if not already set
        if (-not $env:ANDROID_HOME) {
            $defaultSdk = Join-Path $env:USERPROFILE "android-sdk"
            if (Test-Path (Join-Path $defaultSdk "cmdline-tools\latest\bin\sdkmanager.bat")) {
                $env:ANDROID_HOME = $defaultSdk
            } else {
                # Try reading the persisted user env var (set by Install-AndroidSdk)
                $persistedHome = [System.Environment]::GetEnvironmentVariable("ANDROID_HOME", "User")
                if ($persistedHome -and (Test-Path $persistedHome)) {
                    $env:ANDROID_HOME = $persistedHome
                }
            }
        }
        if ($env:ANDROID_HOME) {
            $env:PATH = "$env:ANDROID_HOME\cmdline-tools\latest\bin;$env:ANDROID_HOME\platform-tools;$env:PATH"
            Write-Ok "ANDROID_HOME = $env:ANDROID_HOME"
        } else {
            Write-Warn "ANDROID_HOME is not set — APK build may fail if sdkmanager is not on PATH."
        }

        if (Read-YesNo "Build the Android APK?" "Y") {
            Write-Info "Building Android APK..."

            # Bootstrap gradle-wrapper.jar if missing (it is gitignored)
            $wrapperJar = Join-Path $androidDir "gradle\wrapper\gradle-wrapper.jar"
            $skipApk = $false
            if (-not (Test-Path $wrapperJar)) {
                Write-Info "Downloading gradle-wrapper.jar..."
                $wrapperDir = Join-Path $androidDir "gradle\wrapper"
                New-Item -ItemType Directory -Path $wrapperDir -Force | Out-Null
                $wrapperUrl = "https://raw.githubusercontent.com/gradle/gradle/v8.7.0/gradle/wrapper/gradle-wrapper.jar"
                try {
                    Invoke-FileDownload -Url $wrapperUrl -OutPath $wrapperJar
                    Write-Ok "gradle-wrapper.jar downloaded"
                } catch {
                    Write-Warn "Could not download gradle-wrapper.jar — APK build skipped."
                    Write-Warn "  Download manually: $wrapperUrl"
                    $skipApk = $true
                }
            }

            if (-not $skipApk) {
                Set-Location $androidDir
                $gradlew = Join-Path $androidDir "gradlew.bat"
                if (Test-Path $gradlew) {
                    try {
                        Invoke-NativeBuild -What "gradlew assembleDebug" -Tail 5 -Command { & $gradlew assembleDebug }
                        $apkPath = Join-Path $androidDir "app\build\outputs\apk\debug\app-debug.apk"
                        if (Test-Path $apkPath) {
                            Write-Ok "APK built -> $apkPath"
                        } else {
                            Write-Ok "APK built"
                        }
                    } catch {
                        Write-Warn "APK build failed: $_"
                    }
                } else {
                    Write-Warn "gradlew.bat not found in android directory"
                }
                Set-Location $ScriptDir
            }
        } else {
            Write-Info "Skipping Android build."
        }
    } else {
        if (-not (Test-CommandExists "javac")) {
            Write-Info "Java JDK not found — skipping Android APK build (install Java 17 to enable)"
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
    Write-Host "  Start:    schtasks /Run /TN SimplePhotosServer"
    Write-Host "  Stop:     Task Manager -> Details: kill powershell.exe (watchdog), then simple-photos-server.exe"
    Write-Host "  Disable:  schtasks /End /TN SimplePhotosServer  then  schtasks /Change /TN SimplePhotosServer /DISABLE"
    Write-Host "  Manual:   .\server\target\release\simple-photos-server.exe"
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
