#!/usr/bin/env bash
# =============================================================================
# zero-copy-process.sh — GPU-accelerated zero-copy video processing
# =============================================================================
#
# USAGE:
#   # Passthrough (sanity check — decode → re-encode only):
#   ./zero-copy-process.sh --input input.mp4 --output output.mp4
#
#   # Zero-copy pixel crop:
#   ./zero-copy-process.sh --input input.mp4 --output out.mp4 \
#       --crop 960,0,1920,1080
#
#   # VR180: crop front hemisphere + inject spherical metadata.
#   # --lon-center matches preview_crops.sh degrees (0 = Insta360 X5 front).
#   ./zero-copy-process.sh --input 360.mp4 --output 180vr.mp4 \
#       --vr180 --lon-center 0
#
#   # VR180 at a custom longitude (from preview_crops.sh contact sheet):
#   ./zero-copy-process.sh --input 360.mp4 --output 180vr.mp4 \
#       --vr180 --lon-center 180
#
# PREREQUISITES:
#   cargo (rustup)        — https://rustup.rs
#   ffprobe               — comes with ffmpeg (brew install ffmpeg)
#   uv or python3         — for inject_180_metadata.py
#   spatial-media library — uv add "spatial-media @ git+https://github.com/google/spatial-media.git"
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="$REPO_ROOT/target/release/zero-copy-utils"
INJECT_SCRIPT="$SCRIPT_DIR/inject_180_metadata.py"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info()  { echo -e "${BLUE}[INFO]${NC}  $*"; }
ok()    { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
err()   { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }

usage() {
    echo ""
    echo "Usage: $0 --input <input.mp4> --output <output.mp4> [options]"
    echo ""
    echo "  --input       Input HEVC MP4 file (required)"
    echo "  --output      Output MP4 file (required)"
    echo "  --crop x,y,w,h  Pixel crop (optional; passthrough if omitted)"
    echo "  --vr180         Crop front hemisphere + inject 180° VR metadata"
    echo "  --lon-center N  Longitude centre in degrees, same scale as preview_crops.sh"
    echo "                  (0 = Insta360 X5 front, default 0). Only used with --vr180."
    echo ""
    exit 1
}

INPUT=""
OUTPUT=""
CROP=""
VR180=0
LON_CENTER=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --input)      INPUT="$2";      shift 2 ;;
        --output)     OUTPUT="$2";     shift 2 ;;
        --crop)       CROP="$2";       shift 2 ;;
        --vr180)      VR180=1;         shift   ;;
        --lon-center) LON_CENTER="$2"; shift 2 ;;
        -h|--help) usage ;;
        *) err "Unknown argument: $1" ;;
    esac
done

[[ -z "$INPUT"  ]] && { warn "--input is required";  usage; }
[[ -z "$OUTPUT" ]] && { warn "--output is required"; usage; }
[[ ! -f "$INPUT" ]] && err "Input file not found: $INPUT"

# ── Build binary if needed ────────────────────────────────────────────────────
if [[ ! -x "$BINARY" ]]; then
    info "Binary not found — building zero-copy-utils (release)..."
    cd "$REPO_ROOT"
    cargo build --release -p zero-copy-utils
    ok "Build complete."
fi

# ── Dispatch ──────────────────────────────────────────────────────────────────
if [[ "$VR180" -eq 1 ]]; then
    [[ -n "$CROP" ]] && err "--crop and --vr180 are mutually exclusive"
    [[ -f "$INJECT_SCRIPT" ]] || err "inject_180_metadata.py not found at $INJECT_SCRIPT"

    # Probe source dimensions (needed for metadata injection).
    SRC_W=$(ffprobe -v error -select_streams v:0 \
        -show_entries stream=width -of csv=p=0 "$INPUT" | cut -d, -f1)
    SRC_H=$(ffprobe -v error -select_streams v:0 \
        -show_entries stream=height -of csv=p=0 "$INPUT" | cut -d, -f1)
    info "Source: ${SRC_W}×${SRC_H}"

    # Compute crop dimensions (180° × 180°).
    CROP_W=$(( SRC_W / 2 ))
    CROP_H=$(( SRC_H ))   # fov_v=180° → full height for equirectangular

    # Compute x_offset using the same formula as pipeline.rs SphericalCrop:
    #   center_px = lon_center / 360 * src_width
    #   x_offset  = round(center_px - crop_w / 2)  mod src_width
    # Use awk for the floating-point arithmetic, then normalise with bash modulo.
    X_OFF_RAW=$(awk "BEGIN { printf \"%d\", int(($LON_CENTER / 360.0 * $SRC_W) - ($CROP_W / 2.0) + 0.5) }")
    X_OFF=$(( ((X_OFF_RAW % SRC_W) + SRC_W) % SRC_W ))
    Y_OFF=$(( (SRC_H - CROP_H) / 2 ))

    info "VR180 crop: lon_center=${LON_CENTER}° → x_offset=${X_OFF}  ${CROP_W}×${CROP_H}"

    # Step 1: GPU crop.
    TEMP_CROPPED="${OUTPUT%.mp4}_cropped_temp.mp4"
    info "Step 1/2 — GPU crop..."
    "$BINARY" spherical-crop \
        --input      "$INPUT"   \
        --output     "$TEMP_CROPPED" \
        --lon-center "$LON_CENTER"
    ok "Crop complete → $TEMP_CROPPED"

    # Step 2: Inject 180° spherical metadata.
    info "Step 2/2 — Injecting 180° metadata..."
    if command -v uv >/dev/null 2>&1; then
        PYTHON_RUN="uv run"
    else
        command -v python3 >/dev/null 2>&1 || err "python3 not found and uv not installed"
        PYTHON_RUN="python3"
    fi

    $PYTHON_RUN "$INJECT_SCRIPT" \
        "$TEMP_CROPPED" "$OUTPUT" \
        --full-width  "$SRC_W"  \
        --full-height "$SRC_H"  \
        --x-offset    "$X_OFF"  \
        --top-offset  "$Y_OFF"
    ok "Metadata injected → $OUTPUT"

    rm -f "$TEMP_CROPPED"
    info "Removed temp file: $TEMP_CROPPED"

elif [[ -n "$CROP" ]]; then
    IFS=',' read -r X_OFF Y_OFF CROP_W CROP_H <<< "$CROP"
    [[ -z "$X_OFF" || -z "$Y_OFF" || -z "$CROP_W" || -z "$CROP_H" ]] && \
        err "--crop must be four comma-separated integers: x_offset,y_offset,width,height"

    info "Mode: zero-copy crop  x_offset=${X_OFF}  y_offset=${Y_OFF}  ${CROP_W}×${CROP_H}"
    "$BINARY" crop \
        --input    "$INPUT"   \
        --output   "$OUTPUT"  \
        --x-offset "$X_OFF"   \
        --y-offset "$Y_OFF"   \
        --crop-w   "$CROP_W"  \
        --crop-h   "$CROP_H"

else
    info "Mode: passthrough (decode → re-encode sanity check)"
    "$BINARY" passthrough --input "$INPUT" --output "$OUTPUT"
fi

ok "Done → $OUTPUT"
