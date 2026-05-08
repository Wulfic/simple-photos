# =============================================================================
#  fetch-assets.ps1 — Simple Photos
#
#  Two responsibilities, selected by switch:
#    -GenerateConfig   Generate %ProgramData%\SimplePhotos\config.toml on first
#                      install (idempotent — preserves an existing config).
#    (default)         Download ONNX models + GeoNames dataset into the data dir.
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

function Write-Toml {
    param([string]$Path, [string]$Secret)

    $storage = (Join-Path $DataDir 'storage') -replace '\\', '\\\\'
    $db      = (Join-Path $DataDir 'db\simple-photos.db') -replace '\\', '\\\\'
    $web     = (Join-Path $InstallDir 'web') -replace '\\', '\\\\'

    $cfg = @"
[server]
host = "0.0.0.0"
port = 3000
base_url = "http://localhost:3000"
trust_proxy = false

[database]
path = "$db"
max_connections = 16

[storage]
# Default scaffold path. The first-run setup wizard (web UI) is where the
# operator chooses the final photo storage root.
root = "$storage"
default_quota_bytes = 10737418240
max_blob_size_bytes = 5368709120

[auth]
jwt_secret = "$Secret"
access_token_ttl_secs = 3600
refresh_token_ttl_days = 30
allow_registration = true
bcrypt_cost = 12

[web]
static_root = "$web"

[backup]

[tls]
enabled = false
"@
    Set-Content -Path $Path -Value $cfg -Encoding UTF8
}

# ── Branch 1: config generation (first install only) ──────────────────────
if ($GenerateConfig) {
    if (-not (Test-Path $DataDir)) {
        New-Item -ItemType Directory -Path $DataDir -Force | Out-Null
    }
    $cfgPath = Join-Path $DataDir 'config.toml'
    if (Test-Path $cfgPath) {
        Write-Info "config.toml already exists at $cfgPath — preserving."
        exit 0
    }
    $secret = Get-RandomHex 32
    Write-Toml -Path $cfgPath -Secret $secret
    Write-Info "Generated $cfgPath with a random JWT secret."
    exit 0
}

# ── Branch 2: asset download ──────────────────────────────────────────────
$models = Join-Path $DataDir 'models'
if (-not (Test-Path $models)) { New-Item -ItemType Directory -Path $models -Force | Out-Null }

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

Write-Info "done."
