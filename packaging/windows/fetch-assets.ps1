# =============================================================================
#  fetch-assets.ps1 -- Simple Photos
#
#  Three responsibilities, selected by switch:
#    -GenerateConfig   Generate %ProgramData%\SimplePhotos\config.toml on first
#                      install (idempotent -- preserves existing values, only
#                      appends missing sections so upgrades pick up new
#                      [ai] / [geo] / [scan] / [transcode] defaults).
#    (default)         Download ONNX models + GeoNames dataset + ffmpeg.exe
#                      into the data dir and {app}\bin.
#
#  Invoked by the Inno Setup [Run] section.
# =============================================================================

[CmdletBinding()]
param(
    [switch]$GenerateConfig,
    [string]$InstallDir = "$env:ProgramFiles\SimplePhotos",
    [string]$DataDir    = "$env:ProgramData\SimplePhotos",
    # Release version (e.g. "1.3.44"). Used to fetch the matching Android APK
    # from the GitHub release so the web UI "Download APK" button works on a
    # packaged install. Passed by the Inno Setup [Run] section as {#SP_VERSION}.
    [string]$Version    = ""
)

$ErrorActionPreference = 'Stop'

function Write-Info  { param([string]$Msg) Write-Host "[fetch-assets] $Msg" }
function Write-Warn2 { param([string]$Msg) Write-Warning "[fetch-assets] $Msg" }

function Get-RandomHex {
    param([int]$Bytes = 32)
    $buf = New-Object byte[] $Bytes
    [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($buf)
    -join ($buf | ForEach-Object { '{0:x2}' -f $_ })
}

# Build the full config.toml body. All Windows paths are emitted as TOML
# *literal strings* (single-quoted) so we never have to escape backslashes.
function Build-FullToml {
    param([string]$Secret)

    $storage = Join-Path $DataDir 'storage'
    $db      = Join-Path $DataDir 'db\simple-photos.db'
    $web     = Join-Path $InstallDir 'web'
    $models  = Join-Path $DataDir 'models'
    $geoFile = Join-Path $DataDir 'cities500.txt'

    # Plain string array joined with CRLF -- avoids PS 5.1 here-string
    # encoding pitfalls observed with the Inno Setup [Run] launcher.
    $lines = @(
        '[server]',
        'host = "0.0.0.0"',
        'port = 8080',
        'base_url = "http://localhost:8080"',
        'trust_proxy = false',
        '',
        '[database]',
        ("path = '{0}'" -f $db),
        'max_connections = 16',
        '',
        '[storage]',
        '# Default scaffold path. The first-run setup wizard (web UI) is where the',
        '# operator chooses the final photo storage root.',
        ("root = '{0}'" -f $storage),
        'default_quota_bytes = 10737418240',
        'max_blob_size_bytes = 5368709120',
        '',
        '[auth]',
        ('jwt_secret = "{0}"' -f $Secret),
        'access_token_ttl_secs = 3600',
        'refresh_token_ttl_days = 30',
        'allow_registration = true',
        'bcrypt_cost = 12',
        '',
        '[web]',
        ("static_root = '{0}'" -f $web),
        '',
        '[backup]',
        '',
        '[tls]',
        'enabled = false',
        '',
        '[scan]',
        '# Background storage scan cadence (seconds). 0 disables.',
        'auto_scan_interval_secs = 300',
        '',
        '[ai]',
        '# AI face & object recognition. OFF by default — matches the server',
        '# default and the Linux/Debian package so behaviour is identical',
        '# across platforms. The admin opts in from Settings -> AI (or by',
        '# flipping `enabled` here). model_dir/gpu are pre-wired so enabling',
        '# is a one-line change with the bundled ONNX models.',
        'enabled = false',
        'gpu_preferred = true',
        'allow_heuristic_fallback = false',
        ("model_dir = '{0}'" -f $models),
        '',
        '[geo]',
        '# Reverse geocoding via the offline GeoNames cities500 dataset. OFF by',
        '# default — matches the server default and the Linux/Debian package.',
        '# The fetch-assets step still downloads the dataset so enabling geo',
        '# (here or in Settings) works immediately without re-running setup.',
        'enabled = false',
        ("dataset_path = '{0}'" -f $geoFile),
        '',
        '[transcode]',
        '# Bundled ffmpeg.exe lives in {app}\bin (added to the service PATH).',
        '# CPU fallback is always allowed so missing GPU drivers do not stall',
        '# the conversion pipeline.',
        'gpu_enabled = true',
        'gpu_fallback_to_cpu = true',
        ''
    )
    [string]::Join("`r`n", $lines)
}

function Write-Utf8NoBom {
    param([string]$Path, [string]$Content)
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Content, $utf8NoBom)
}

# Append any missing [section] blocks to an existing config. We deliberately
# do *not* edit values inside sections the operator already has -- that
# preserves manual customisations across upgrades.
function Update-ExistingToml {
    param([string]$Path, [string]$Secret)

    $existing = Get-Content -Raw -LiteralPath $Path
    $full     = Build-FullToml -Secret $Secret

    $needed   = @('[scan]', '[ai]', '[geo]', '[transcode]')
    $appended = @()
    foreach ($hdr in $needed) {
        if ($existing -notmatch [regex]::Escape($hdr)) {
            # Extract just this section out of $full (header through next
            # blank line before the following section header, or EOF).
            $pattern = "(?ms)^" + [regex]::Escape($hdr) + ".*?(?=^\[|\z)"
            $m = [regex]::Match($full, $pattern)
            if ($m.Success) {
                $appended += $m.Value.TrimEnd()
            }
        }
    }
    if ($appended.Count -eq 0) {
        Write-Info "config.toml already has [scan] [ai] [geo] [transcode] -- nothing to do."
        return
    }
    if ($existing -notmatch "(\r?\n)\z") { $existing += "`r`n" }
    $patched = $existing + "`r`n" + ([string]::Join("`r`n`r`n", $appended)) + "`r`n"
    Write-Utf8NoBom -Path $Path -Content $patched
    Write-Info ("Patched config.toml -- appended {0} missing section(s)." -f $appended.Count)
}

# -- Branch 1: config generation / migration (always idempotent) ----------
if ($GenerateConfig) {
    if (-not (Test-Path $DataDir)) {
        New-Item -ItemType Directory -Path $DataDir -Force | Out-Null
    }
    $cfgPath = Join-Path $DataDir 'config.toml'
    if (Test-Path $cfgPath) {
        Write-Info "config.toml exists at $cfgPath -- patching missing sections."
        Update-ExistingToml -Path $cfgPath -Secret (Get-RandomHex 32)
        exit 0
    }
    $secret = Get-RandomHex 32
    Write-Utf8NoBom -Path $cfgPath -Content (Build-FullToml -Secret $secret)
    Write-Info "Generated $cfgPath with a random JWT secret."
    exit 0
}

# -- Branch 2: asset download ----------------------------------------------
$models = Join-Path $DataDir 'models'
if (-not (Test-Path $models)) { New-Item -ItemType Directory -Path $models -Force | Out-Null }

$binDir = Join-Path $InstallDir 'bin'
if (-not (Test-Path $binDir)) { New-Item -ItemType Directory -Path $binDir -Force | Out-Null }

function Get-File {
    param([string]$Url, [string]$OutPath)
    if ((Test-Path $OutPath) -and (Get-Item $OutPath).Length -gt 0) {
        Write-Info "skip $($OutPath | Split-Path -Leaf) (already present)"
        return
    }
    $part = "$OutPath.part"
    Write-Info "get  $($OutPath | Split-Path -Leaf)"
    try {
        Invoke-WebRequest -Uri $Url -OutFile $part -UseBasicParsing
        Move-Item -Path $part -Destination $OutPath -Force
    } catch {
        Remove-Item $part -ErrorAction SilentlyContinue
        Write-Warn2 "download failed: $($_.Exception.Message)"
    }
}

Get-File "https://huggingface.co/immich-app/buffalo_l/resolve/main/detection/model.onnx" `
         (Join-Path $models 'det_10g.onnx')
Get-File "https://huggingface.co/immich-app/buffalo_l/resolve/main/recognition/model.onnx" `
         (Join-Path $models 'w600k_r50.onnx')
Get-File "https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/raw/master/models/onnx/version-RFB-320.onnx" `
         (Join-Path $models 'ultraface-RFB-320.onnx')
Get-File "https://github.com/onnx/models/raw/refs/heads/main/validated/vision/classification/mobilenet/model/mobilenetv2-12.onnx" `
         (Join-Path $models 'mobilenetv2-12.onnx')

# GeoNames cities500 zip
$geo = Join-Path $DataDir 'cities500.txt'
if (-not (Test-Path $geo) -or (Get-Item $geo).Length -eq 0) {
    $tmp = Join-Path $env:TEMP "cities500-$([guid]::NewGuid()).zip"
    try {
        Write-Info "get  cities500.zip"
        Invoke-WebRequest -Uri "https://download.geonames.org/export/dump/cities500.zip" `
                          -OutFile $tmp -UseBasicParsing
        $extract = Join-Path $env:TEMP "cities500-$([guid]::NewGuid())"
        Expand-Archive -Path $tmp -DestinationPath $extract -Force
        Move-Item -Path (Join-Path $extract 'cities500.txt') -Destination $geo -Force
        Remove-Item -Recurse -Force $extract
    } catch {
        Write-Warn2 "GeoNames download failed: $($_.Exception.Message)"
    } finally {
        Remove-Item $tmp -ErrorAction SilentlyContinue
    }
}

Get-File "https://download.geonames.org/export/dump/admin1CodesASCII.txt" `
         (Join-Path $DataDir 'admin1CodesASCII.txt')

# -- ffmpeg.exe (video thumbnails + transcoding) ---------------------------
# BtbN nightly GPL build -- includes ffmpeg.exe + ffprobe.exe statically.
$ffmpegExe  = Join-Path $binDir 'ffmpeg.exe'
$ffprobeExe = Join-Path $binDir 'ffprobe.exe'
if (-not ((Test-Path $ffmpegExe) -and (Test-Path $ffprobeExe))) {
    $tmp = Join-Path $env:TEMP "ffmpeg-$([guid]::NewGuid()).zip"
    try {
        Write-Info "get  ffmpeg (windows-x64 master-latest-gpl)"
        $url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
        Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing
        $extract = Join-Path $env:TEMP "ffmpeg-$([guid]::NewGuid())"
        Expand-Archive -Path $tmp -DestinationPath $extract -Force
        $found = Get-ChildItem -Path $extract -Recurse -Filter 'ffmpeg.exe' | Select-Object -First 1
        if ($found) {
            Copy-Item -Path $found.FullName -Destination $ffmpegExe -Force
            $probeSrc = Join-Path $found.Directory.FullName 'ffprobe.exe'
            if (Test-Path $probeSrc) {
                Copy-Item -Path $probeSrc -Destination $ffprobeExe -Force
            }
            Write-Info "installed ffmpeg.exe -> $ffmpegExe"
        } else {
            Write-Warn2 "ffmpeg.exe not found inside the downloaded archive."
        }
        Remove-Item -Recurse -Force $extract
    } catch {
        Write-Warn2 "ffmpeg download failed: $($_.Exception.Message)"
    } finally {
        Remove-Item $tmp -ErrorAction SilentlyContinue
    }
} else {
    Write-Info "skip ffmpeg.exe (already present)"
}

# -- Android APK (in-app "Download APK" button) ----------------------------
# The server serves the APK from {InstallDir}\downloads\simple-photos.apk
# (see server/src/downloads/handlers.rs). Pull the versioned APK published on
# the GitHub release so the setup-wizard download works on a packaged install.
# Best-effort: a failure here must never abort the install — the web UI shows a
# helpful message if the APK is absent, and the user can re-run this script.
$downloadsDir = Join-Path $InstallDir 'downloads'
if (-not (Test-Path $downloadsDir)) { New-Item -ItemType Directory -Path $downloadsDir -Force | Out-Null }
$apkOut = Join-Path $downloadsDir 'simple-photos.apk'
if ((Test-Path $apkOut) -and (Get-Item $apkOut).Length -gt 0) {
    Write-Info "skip simple-photos.apk (already present)"
} elseif ([string]::IsNullOrWhiteSpace($Version)) {
    Write-Warn2 "no -Version supplied; skipping APK download (web UI 'Download APK' will be unavailable until the APK is placed in $downloadsDir)."
} else {
    $apkUrl = "https://github.com/Wulfic/simple-photos/releases/download/v$Version/simple-photos-$Version.apk"
    $part = "$apkOut.part"
    Write-Info "get  simple-photos.apk (release v$Version)"
    try {
        Invoke-WebRequest -Uri $apkUrl -OutFile $part -UseBasicParsing
        if ((Get-Item $part).Length -lt 1MB) { throw "downloaded APK is suspiciously small" }
        Move-Item -Path $part -Destination $apkOut -Force
        Write-Info "installed APK -> $apkOut"
    } catch {
        Remove-Item $part -ErrorAction SilentlyContinue
        Write-Warn2 "APK download failed: $($_.Exception.Message)"
    }
}

Write-Info "done."
