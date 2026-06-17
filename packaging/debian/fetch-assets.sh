#!/usr/bin/env bash
# fetch-assets.sh — download AI models + GeoNames dataset post-install.
#
# Run as the simple-photos user:
#   sudo -u simple-photos /usr/share/simple-photos/fetch-assets.sh
#
# These assets are NOT bundled in the .deb (~225 MB combined).  Without
# them the server runs in degraded mode: face/object recognition disabled,
# reverse-geocoding leaves geo_city/geo_country empty.

set -euo pipefail

DATA_DIR="${DATA_DIR:-/var/lib/simple-photos}"
MODELS="$DATA_DIR/models"
GEO="$DATA_DIR/cities500.txt"
ADMIN1="$DATA_DIR/admin1CodesASCII.txt"
DOWNLOADS="$DATA_DIR/downloads"

# Release version, stamped in by the CI .deb build (sed @SP_VERSION@ → x.y.z).
# Used to fetch the matching Android APK. Falls back to env SP_VERSION for
# local/manual runs; if neither is set the APK download is skipped (best-effort).
SP_APK_VERSION="${SP_VERSION:-@SP_VERSION@}"

mkdir -p "$MODELS" "$DATA_DIR" "$DOWNLOADS"

dl() {
    local out="$1" url="$2"
    if [ -s "$out" ]; then
        echo "[skip] $(basename "$out") already present"
        return 0
    fi
    echo "[get]  $(basename "$out")"
    curl -fL --retry 3 --output "$out.part" "$url"
    mv "$out.part" "$out"
}

# Face models live on a pinned, version-independent "assets-models" GitHub
# release (github.com) so the install does NOT depend on HuggingFace's Xet CDN
# (cas-bridge.xethub.co), which some networks can't resolve (issue #5b). The
# fixed tag works for every build, including local/unstamped ones. Try the
# mirror first (by exact name), then fall back to the HuggingFace source.
MODEL_MIRROR_BASE="https://github.com/Wulfic/simple-photos/releases/download/assets-models"

dl_model() {
    local out="$1" name="$2" fallback="$3"
    if [ -s "$out" ]; then
        echo "[skip] $name already present"
        return 0
    fi
    echo "[get]  $name (models mirror)"
    if curl -fL --retry 5 --retry-all-errors --output "$out.part" "$MODEL_MIRROR_BASE/$name"; then
        mv "$out.part" "$out"
        return 0
    fi
    rm -f "$out.part"
    echo "[warn] models mirror for $name unavailable — falling back to HuggingFace"
    echo "[get]  $name (source)"
    curl -fL --retry 3 --output "$out.part" "$fallback"
    mv "$out.part" "$out"
}

dl_model "$MODELS/det_10g.onnx"   "det_10g.onnx"   "https://huggingface.co/immich-app/buffalo_l/resolve/main/detection/model.onnx"
dl_model "$MODELS/w600k_r50.onnx" "w600k_r50.onnx" "https://huggingface.co/immich-app/buffalo_l/resolve/main/recognition/model.onnx"
dl "$MODELS/ultraface-RFB-320.onnx"  "https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/raw/master/models/onnx/version-RFB-320.onnx"
dl "$MODELS/mobilenetv2-12.onnx"     "https://github.com/onnx/models/raw/refs/heads/main/validated/vision/classification/mobilenet/model/mobilenetv2-12.onnx"

# GeoNames cities500 — extract from zip.
if [ ! -s "$GEO" ]; then
    tmp="$(mktemp -d)"
    trap "rm -rf '$tmp'" EXIT
    echo "[get]  cities500.zip"
    curl -fL --retry 3 -o "$tmp/cities500.zip" \
        "https://download.geonames.org/export/dump/cities500.zip"
    unzip -p "$tmp/cities500.zip" cities500.txt > "$GEO"
fi

dl "$ADMIN1" "https://download.geonames.org/export/dump/admin1CodesASCII.txt" || \
    echo "[warn] admin1 download failed — state names will fall back to 2-char codes"

# ── Android APK ───────────────────────────────────────────────────────────
# Served by the web UI "Download APK" button from $DATA_DIR/downloads (the
# server's working dir is $DATA_DIR, see simple-photos.service).
#
# Strategy (issue #4): the APK is BUNDLED in the package and seeded into
# $DOWNLOADS by the postinst, so the button works immediately and offline.
# Here we try to REFRESH it from the matching GitHub release (in case the
# release carries a rebuilt artifact), and on any failure fall back to the
# bundled copy. Best-effort: never abort install.
APK="$DOWNLOADS/simple-photos.apk"
BUNDLED_APK="/usr/share/simple-photos/simple-photos.apk"

if [ -z "$SP_APK_VERSION" ] || [ "$SP_APK_VERSION" = "@SP_VERSION@" ]; then
    echo "[skip] no release version stamped — keeping bundled APK (no refresh)"
else
    APK_URL="https://github.com/Wulfic/simple-photos/releases/download/v${SP_APK_VERSION}/simple-photos-${SP_APK_VERSION}.apk"
    echo "[get]  simple-photos.apk (release v${SP_APK_VERSION})"
    if curl -fL --retry 3 --output "$APK.part" "$APK_URL"; then
        mv "$APK.part" "$APK"
        echo "[ok]   APK refreshed from release -> $APK"
    else
        rm -f "$APK.part"
        echo "[warn] APK refresh from release failed — falling back to bundled APK"
    fi
fi

# First-boot / fallback guarantee: if no APK is present (refresh skipped or
# failed and postinst seeding didn't run, e.g. a systemd-less container), use
# the copy bundled in the package.
if [ ! -s "$APK" ] && [ -s "$BUNDLED_APK" ]; then
    cp "$BUNDLED_APK" "$APK"
    echo "[ok]   APK provided from bundled package copy -> $APK"
fi
if [ ! -s "$APK" ]; then
    echo "[warn] no APK available (no bundled copy, release refresh failed) — web UI 'Download APK' unavailable"
fi

echo "[done] AI models, geo data and Android APK installed under $DATA_DIR"
echo "[next] sudo systemctl restart simple-photos"

# ── NVIDIA / CUDA runtime check ───────────────────────────────────────────────
echo ""
if command -v nvidia-smi >/dev/null 2>&1; then
    echo "[gpu]  NVIDIA GPU detected: $(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1)"
    if ldconfig -p 2>/dev/null | grep -q 'libcudart'; then
        echo "[gpu]  CUDA runtime present — GPU inference will be used automatically."
    else
        echo "[gpu]  CUDA runtime NOT found. Install it for GPU-accelerated AI inference:"
        echo "         sudo apt-get install nvidia-cuda-toolkit"
        echo "       Or use the NVIDIA-published packages (recommended, more up to date):"
        echo "         https://developer.nvidia.com/cuda-downloads"
    fi
else
    echo "[gpu]  No NVIDIA GPU detected — running CPU inference (slower)."
    echo "       If you add a GPU later, install nvidia-cuda-toolkit and re-run this script."
fi

# ── FFmpeg check ──────────────────────────────────────────────────────────────
echo ""
if command -v ffmpeg >/dev/null 2>&1; then
    echo "[ffmpeg] $(ffmpeg -version 2>&1 | head -1)"
else
    echo "[ffmpeg] ffmpeg not found — video transcoding will be unavailable."
    echo "         Install with: sudo apt-get install ffmpeg"
fi
