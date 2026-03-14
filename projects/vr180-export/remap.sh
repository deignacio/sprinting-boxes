#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CACHE_DIR="${HOME}/.cache/vr180-export"

usage() {
    echo "Usage: $0 <input.mp4> <output.mp4> [--yaw <degrees>] [--pitch <degrees>] [--roll <degrees>]"
    echo "   or: $0 --input <input.mp4> --output <output.mp4> [--yaw <degrees>] [--pitch <degrees>] [--roll <degrees>]"
    echo ""
    echo "  --yaw     Horizontal rotation in degrees (default: 0)"
    echo "  --pitch   Vertical rotation in degrees (default: 0)"
    echo "  --roll    Roll rotation in degrees (default: 0)"
    exit 1
}

INPUT=""
OUTPUT=""
YAW=0
PITCH=0
ROLL=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --input)  INPUT="$2";  shift 2 ;;
        --output) OUTPUT="$2"; shift 2 ;;
        --yaw)    YAW="$2";    shift 2 ;;
        --pitch)  PITCH="$2";  shift 2 ;;
        --roll)   ROLL="$2";   shift 2 ;;
        -*)       usage ;;
        *)
            if [[ -z "$INPUT" ]]; then
                INPUT="$1"
            elif [[ -z "$OUTPUT" ]]; then
                OUTPUT="$1"
            else
                usage
            fi
            shift
            ;;
    esac
done

[[ -z "$INPUT"  ]] && { echo "Error: --input is required";  usage; }
[[ -z "$OUTPUT" ]] && { echo "Error: --output is required"; usage; }
[[ ! -f "$INPUT" ]] && { echo "Error: input file not found: $INPUT"; exit 1; }

# Probe source resolution
SRC_W=$(ffprobe -v quiet -select_streams v:0 \
    -show_entries stream=width -of default=noprint_wrappers=1:nokey=1 "$INPUT")
SRC_H=$(ffprobe -v quiet -select_streams v:0 \
    -show_entries stream=height -of default=noprint_wrappers=1:nokey=1 "$INPUT")
OUT_SIZE=$((SRC_W / 2))

echo "==> Source: ${SRC_W}x${SRC_H} → fisheye ${OUT_SIZE}x${OUT_SIZE}"

# LUT cache key
CACHE_KEY="${SRC_W}x${SRC_H}_y${YAW}_p${PITCH}_r${ROLL}"
XMAP="${CACHE_DIR}/${CACHE_KEY}.xmap.pgm"
YMAP="${CACHE_DIR}/${CACHE_KEY}.ymap.pgm"

mkdir -p "$CACHE_DIR"

if [[ -f "$XMAP" && -f "$YMAP" ]]; then
    echo "==> [1/3] LUT cache hit: $CACHE_KEY"
else
    echo "==> [1/3] Generating remap LUT (${OUT_SIZE}x${OUT_SIZE})..."
    cd "$SCRIPT_DIR"
    uv run gen_lut.py \
        --src-w "$SRC_W" --src-h "$SRC_H" \
        --out-size "$OUT_SIZE" \
        --yaw "$YAW" --pitch "$PITCH" --roll "$ROLL" \
        --xmap "$XMAP" --ymap "$YMAP"
fi

TMPFILE="$(dirname "$OUTPUT")/.vr180_tmp_$(basename "$OUTPUT")"
trap 'rm -f "$TMPFILE"' EXIT

echo "==> [2/3] Reprojecting equirectangular → mono fisheye via remap..."
ffmpeg -y \
    -i "$INPUT" \
    -i "$XMAP" \
    -i "$YMAP" \
    -filter_complex "[0:v][1:v][2:v]remap=fill=black" \
    -c:v hevc_videotoolbox -q:v 65 -tag:v hvc1 \
    -c:a copy \
    -movflags +faststart \
    "$TMPFILE"

echo "==> [3/3] Injecting mono VR180 spatial metadata..."
cd "$SCRIPT_DIR"
uv run inject_metadata.py "$TMPFILE" "$OUTPUT"

echo "==> Verifying output..."
ffprobe -v quiet -show_entries stream_tags:stream_side_data \
    -of default "$OUTPUT" 2>&1 | grep -Ei "spherical|stereo|vr180" || true

echo "==> Done: $OUTPUT"
