#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

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

TMPFILE="$(dirname "$OUTPUT")/.vr180_tmp_$(basename "$OUTPUT")"
trap 'rm -f "$TMPFILE"' EXIT

METAL_BINARY="$(cd "$SCRIPT_DIR/../../" 2>/dev/null && pwd)/target/release/vr180-metal"

echo "==> [1/3] Reprojecting equirectangular → mono fisheye (yaw=${YAW}°, pitch=${PITCH}°, roll=${ROLL}°)..."
if [[ -x "$METAL_BINARY" ]]; then
    echo "      (using Metal GPU pipeline)"
    "$METAL_BINARY" \
        --input "$INPUT" \
        --output "$TMPFILE" \
        --yaw "$YAW" \
        --pitch "$PITCH" \
        --roll "$ROLL"
else
    echo "      (Metal binary not found at $METAL_BINARY — using FFmpeg v360 fallback)"
    echo "      (run: cargo build --release -p vr180-metal  to enable GPU acceleration)"
    ffmpeg -y -i "$INPUT" \
        -vf "v360=e:fisheye:ih_fov=360:iv_fov=180:h_fov=180:v_fov=180:yaw=${YAW}:pitch=${PITCH}:roll=${ROLL}" \
        -c:v hevc_videotoolbox -q:v 65 -tag:v hvc1 \
        -c:a copy \
        -movflags +faststart \
        "$TMPFILE"
fi

echo "==> [2/3] Injecting mono VR180 spatial metadata..."
cd "$SCRIPT_DIR"
uv run inject_metadata.py "$TMPFILE" "$OUTPUT"

echo "==> [3/3] Verifying output..."
ffprobe -v quiet -show_entries stream_tags:stream_side_data \
    -of default "$OUTPUT" 2>&1 | grep -Ei "spherical|stereo|vr180" || true

echo "==> Done: $OUTPUT"
