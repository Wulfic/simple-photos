#!/usr/bin/env bash
# Download all ONNX models needed for AI face/object recognition.
#
# After P0-2, the server refuses to silently emit heuristic-only "AI"
# results.  When the model directory is empty (and
# ai.allow_heuristic_fallback is left at its default of false), the
# `/api/ai/status` endpoint reports `degraded_mode: true` and skips face
# clustering / object tagging entirely.
#
# This script primes the model directory with the same models that
# init_face_model / init_object_model would download lazily on first
# use.  Run it once during install (or whenever you wipe `server/models`)
# so the first request after a cold start does not stall on a multi-MB
# download.
#
# Output: $TARGET (default: server/models/)
#
# Models bundled here (all ~Apache-2.0 / MIT-licensed):
#   - det_10g.onnx              SCRFD face detector  (Immich/InsightFace)
#   - w600k_r50.onnx            ArcFace 512-d face embeddings
#   - ultraface-RFB-320.onnx    UltraFace fallback detector
#   - mobilenetv2-12.onnx       ImageNet 1000-class object classifier

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${1:-$REPO_ROOT/server/models}"
mkdir -p "$TARGET"

declare -A MODELS=(
  [det_10g.onnx]="https://huggingface.co/immich-app/buffalo_l/resolve/main/detection/model.onnx"
  [w600k_r50.onnx]="https://huggingface.co/immich-app/buffalo_l/resolve/main/recognition/model.onnx"
  [ultraface-RFB-320.onnx]="https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/raw/master/models/onnx/version-RFB-320.onnx"
  [mobilenetv2-12.onnx]="https://github.com/onnx/models/raw/refs/heads/main/validated/vision/classification/mobilenet/model/mobilenetv2-12.onnx"
)

fetch() {
    local out="$1"
    local url="$2"
    if [[ -s "$out" ]]; then
        echo "[ai] $out already present — skipping"
        return 0
    fi
    echo "[ai] Downloading $(basename "$out") ← $url"
    if command -v curl >/dev/null 2>&1; then
        curl -fL --retry 3 -o "$out.part" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$out.part" "$url"
    else
        echo "ERROR: need curl or wget on PATH" >&2
        exit 1
    fi
    if [[ ! -s "$out.part" ]]; then
        echo "ERROR: download produced empty file: $out.part" >&2
        rm -f "$out.part"
        exit 1
    fi
    mv "$out.part" "$out"
    echo "[ai]   → $(du -h "$out" | cut -f1)"
}

for name in "${!MODELS[@]}"; do
    fetch "$TARGET/$name" "${MODELS[$name]}"
done

echo
echo "[ai] All models present in $TARGET:"
ls -lh "$TARGET" | tail -n +2
echo
echo "[ai] Tip: set SIMPLE_PHOTOS_AI_MODEL_DIR=$TARGET if you ran the"
echo "[ai] server with a non-default ai.model_dir in config.toml."
