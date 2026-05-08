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

mkdir -p "$MODELS" "$DATA_DIR"

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

dl "$MODELS/det_10g.onnx"            "https://huggingface.co/immich-app/buffalo_l/resolve/main/detection/model.onnx"
dl "$MODELS/w600k_r50.onnx"          "https://huggingface.co/immich-app/buffalo_l/resolve/main/recognition/model.onnx"
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

echo "[done] AI models and geo data installed under $DATA_DIR"
echo "[next] sudo systemctl restart simple-photos"
