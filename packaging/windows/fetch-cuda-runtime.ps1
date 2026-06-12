# =============================================================================
#  fetch-cuda-runtime.ps1 -- Simple Photos
#
#  Downloads the NVIDIA CUDA 12 + cuDNN 9 *runtime* DLLs that the ONNX Runtime
#  CUDA execution provider (onnxruntime_providers_cuda.dll) depends on, and
#  places them next to the server binary so GPU AI inference works without a
#  full CUDA Toolkit install.
#
#  Why this exists:
#    The build ships onnxruntime_providers_cuda.dll (~92 MB) but NOT the CUDA
#    runtime it loads at first inference (cudart64_12.dll, cublas64_12.dll,
#    cublasLt64_12.dll, cufft64_11.dll, cudnn*64_9.dll). Without them the EP
#    silently falls back to CPU. These libraries are large (~0.6-1 GB) and
#    carry NVIDIA's redistribution terms, so we fetch them on demand from
#    NVIDIA's official redist server rather than bundling them in the installer.
#
#  ort 2.0.0-rc.12  ==  ONNX Runtime 1.20.x  ==  CUDA 12.x + cuDNN 9.x.
#
#  Usage:
#    .\fetch-cuda-runtime.ps1 -InstallDir "C:\Program Files\SimplePhotos"
#    .\fetch-cuda-runtime.ps1 -Verify        # just report what's present
#
#  Invoked by the Inno Setup [Run] section when the user selects the optional
#  "GPU acceleration" component, and runnable standalone for dev/manual setup.
# =============================================================================

[CmdletBinding()]
param(
    # Directory the server binary lives in; DLLs are copied here so Windows'
    # loader finds them next to onnxruntime_providers_cuda.dll.
    [string]$InstallDir = (Split-Path -Parent $MyInvocation.MyCommand.Path),

    # Pinned redist manifest versions compatible with ONNX Runtime 1.20.
    [string]$CudaVersion  = '12.6.3',
    [string]$CudnnVersion = '9.5.1',

    # Only report which runtime DLLs are present, download nothing.
    [switch]$Verify
)

$ErrorActionPreference = 'Stop'
$ProgressPreference     = 'SilentlyContinue'   # faster Invoke-WebRequest

function Write-Info { param([string]$Msg) Write-Host "[cuda-runtime] $Msg" }
function Write-Warn2 { param([string]$Msg) Write-Warning "[cuda-runtime] $Msg" }

# The DLLs the CUDA EP needs at runtime. Presence of all of these next to the
# binary (or on PATH) means GPU AI can initialise.
$RequiredDlls = @(
    'cudart64_12.dll',
    'cublas64_12.dll',
    'cublasLt64_12.dll',
    'cufft64_11.dll',
    'cudnn64_9.dll'
)

function Test-RuntimePresent {
    param([string]$Dir)
    $missing = @()
    foreach ($d in $RequiredDlls) {
        if (-not (Test-Path (Join-Path $Dir $d))) { $missing += $d }
    }
    return $missing
}

# ---------------------------------------------------------------------------
# Verify-only mode: report and exit.
# ---------------------------------------------------------------------------
if ($Verify) {
    $missing = Test-RuntimePresent -Dir $InstallDir
    if ($missing.Count -eq 0) {
        Write-Info "All CUDA runtime DLLs present in $InstallDir."
        exit 0
    }
    Write-Warn2 ("Missing CUDA runtime DLLs: {0}" -f ($missing -join ', '))
    exit 1
}

if (-not (Test-Path $InstallDir)) {
    throw "InstallDir not found: $InstallDir"
}

$missing = Test-RuntimePresent -Dir $InstallDir
if ($missing.Count -eq 0) {
    Write-Info "CUDA runtime already installed in $InstallDir -- nothing to do."
    exit 0
}
Write-Info ("Need: {0}" -f ($missing -join ', '))

$cudaBase  = 'https://developer.download.nvidia.com/compute/cuda/redist'
$cudnnBase = 'https://developer.download.nvidia.com/compute/cudnn/redist'

# Download a redist component archive (by relative_path from a manifest),
# extract it, and copy every bin\*.dll into $InstallDir.
function Install-RedistComponent {
    param(
        [Parameter(Mandatory)] [string]$BaseUrl,
        [Parameter(Mandatory)] [string]$RelativePath,
        [Parameter(Mandatory)] [string]$DestDir
    )
    $url  = "$BaseUrl/$RelativePath"
    $name = Split-Path $RelativePath -Leaf
    $tmp  = Join-Path $env:TEMP "sp-cuda-$([guid]::NewGuid())"
    New-Item -ItemType Directory -Path $tmp -Force | Out-Null
    $zip  = Join-Path $tmp $name
    try {
        Write-Info "get  $name"
        Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
        Expand-Archive -Path $zip -DestinationPath $tmp -Force
        $dlls = Get-ChildItem -Path $tmp -Recurse -Filter '*.dll' |
                Where-Object { $_.DirectoryName -match '\\bin$' }
        foreach ($f in $dlls) {
            Copy-Item -Path $f.FullName -Destination (Join-Path $DestDir $f.Name) -Force
        }
        Write-Info ("     extracted {0} dll(s)" -f $dlls.Count)
    } finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# Pull a redist manifest (JSON) and return the windows-x86_64 relative_path
# for a named component. cuDNN nests a cuda12/cuda11 variant under the arch;
# CUDA toolkit components expose relative_path directly.
function Get-ComponentRelativePath {
    param(
        [Parameter(Mandatory)] [string]$ManifestUrl,
        [Parameter(Mandatory)] [string]$Component
    )
    $manifest = Invoke-WebRequest -Uri $ManifestUrl -UseBasicParsing | Select-Object -ExpandProperty Content | ConvertFrom-Json
    if (-not $manifest.$Component) {
        throw "Component '$Component' not found in manifest $ManifestUrl"
    }
    $arch = $manifest.$Component.'windows-x86_64'
    if (-not $arch) {
        throw "Component '$Component' has no windows-x86_64 archive in $ManifestUrl"
    }
    if ($arch.relative_path) {
        return $arch.relative_path
    }
    # cuDNN style: windows-x86_64 -> cuda12 -> relative_path
    foreach ($variant in @('cuda12', 'cuda-12', 'cuda12.6')) {
        if ($arch.$variant -and $arch.$variant.relative_path) {
            return $arch.$variant.relative_path
        }
    }
    # Fall back to the first nested object that has a relative_path.
    foreach ($p in $arch.PSObject.Properties) {
        if ($p.Value.relative_path) { return $p.Value.relative_path }
    }
    throw "Could not resolve a windows-x86_64 relative_path for '$Component'."
}

try {
    Write-Info "Downloading CUDA $CudaVersion + cuDNN $CudnnVersion runtime (~0.6-1 GB)..."

    # --- CUDA toolkit runtime components -----------------------------------
    $cudaManifest = "$cudaBase/redistrib_$CudaVersion.json"
    foreach ($component in @('cuda_cudart', 'libcublas', 'libcufft')) {
        $rel = Get-ComponentRelativePath -ManifestUrl $cudaManifest -Component $component
        Install-RedistComponent -BaseUrl $cudaBase -RelativePath $rel -DestDir $InstallDir
    }

    # --- cuDNN runtime -----------------------------------------------------
    $cudnnManifest = "$cudnnBase/redistrib_$CudnnVersion.json"
    $rel = Get-ComponentRelativePath -ManifestUrl $cudnnManifest -Component 'cudnn'
    Install-RedistComponent -BaseUrl $cudnnBase -RelativePath $rel -DestDir $InstallDir
} catch {
    Write-Warn2 "CUDA runtime download failed: $($_.Exception.Message)"
    Write-Warn2 "GPU AI will be unavailable; the server runs fine on CPU."
    exit 1
}

$missing = Test-RuntimePresent -Dir $InstallDir
if ($missing.Count -eq 0) {
    Write-Info "CUDA runtime installed successfully in $InstallDir."
    exit 0
}
Write-Warn2 ("Install incomplete -- still missing: {0}" -f ($missing -join ', '))
exit 1
