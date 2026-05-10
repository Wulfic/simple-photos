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
    [string]$DataDir    = "$env:ProgramData\SimplePhotos"
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
        '# AI tagging / face & object recognition. Heuristic fallback ON so',
        '# tag-based smart features still work when ONNX models are missing.',
        'enabled = true',
        'gpu_preferred = true',
        'allow_heuristic_fallback = true',
        ("model_dir = '{0}'" -f $models),
        '',
        '[geo]',
        '# Reverse geocoding via the offline GeoNames cities500 dataset. The',
        '# fetch-assets step downloads the file alongside this config.',
        'enabled = true',
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

Write-Info "done."
