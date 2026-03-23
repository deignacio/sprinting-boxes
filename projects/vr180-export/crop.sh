#!/usr/bin/env bash
# =============================================================================
# Insta360 X5 — 360° to 180° VR Converter for YouTube
# =============================================================================
#
# WHAT THIS DOES:
#   1. Crops the front hemisphere (left half of equirectangular frame)
#   2. Injects YouTube-compatible 180° spherical metadata
#
# PREREQUISITES:
#   - ffmpeg       → https://ffmpeg.org/download.html
#   - Python 3     → https://www.python.org/downloads/
#   - spatial-media → pip install spatial-media
#                  OR: https://github.com/google/spatial-media/releases
#
# INSTALL PREREQUISITES (macOS with Homebrew):
#   brew install ffmpeg
#   pip3 install spatial-media
#
# INSTALL PREREQUISITES (Ubuntu/Debian):
#   sudo apt install ffmpeg python3-pip
#   pip3 install spatial-media
#
# USAGE:
#   chmod +x convert_360_to_180vr.sh
#   ./convert_360_to_180vr.sh input.mp4
#   ./convert_360_to_180vr.sh input.mp4 0              # default — 0° center longitude
#   ./convert_360_to_180vr.sh input.mp4 90             # rotate 90° right
#   ./convert_360_to_180vr.sh input.mp4 front          # alias for 0°
#   ./convert_360_to_180vr.sh input.mp4 back           # alias for 180°
#   ./convert_360_to_180vr.sh input.mp4 0 1:1          # default square (full 180°×180°)
#   ./convert_360_to_180vr.sh input.mp4 0 16:9         # widescreen (loses ~39° top/bottom)
#   ./convert_360_to_180vr.sh input.mp4 0 4:3          # moderate crop (loses ~22° top/bottom)
#
# Use preview_crops.sh first to pick the right angle visually.
#
# OUTPUT:
#   input_180vr.mp4  — ready to upload directly to YouTube
#
# INSTA360 X5 SUPPORTED SOURCE RESOLUTIONS → OUTPUT:
#   8K  7680×3840  →  3840×3840
#   5.7K 5760×2880 →  2880×2880
#   4K  3840×1920  →  1920×1920
#
# NOTE: Export your .insv file from Insta360 Studio as a 360° equirectangular
# MP4 BEFORE running this script. Do NOT use the reframed/flat export.
# =============================================================================

set -euo pipefail

# ── Colour output ──────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }

# ── Argument handling ──────────────────────────────────────────────────────────
INPUT="${1:-}"
CENTER_ARG="${2:-0}"   # center longitude in degrees (0–359), or "front"/"back" aliases
ASPECT_ARG="${3:-1:1}" # output aspect ratio W:H (default 1:1 = full 180°×180° hemisphere)

[[ -z "$INPUT" ]]   && error "Usage: $0 <input.mp4> [center_longitude_degrees|front|back] [aspect_ratio]"
[[ ! -f "$INPUT" ]] && error "File not found: $INPUT"

# Resolve front/back aliases to degrees
case "$CENTER_ARG" in
  front) CENTER_LON=0   ;;
  back)  CENTER_LON=180 ;;
  ''|*[!0-9]*)
    error "Second argument must be a degree (0–359) or 'front'/'back'. Got: $CENTER_ARG" ;;
  *)     CENTER_LON="$CENTER_ARG" ;;
esac

if [[ "$CENTER_LON" -lt 0 || "$CENTER_LON" -gt 359 ]]; then
  error "Center longitude must be between 0 and 359. Got: $CENTER_LON"
fi

# Parse aspect ratio W:H
if [[ "$ASPECT_ARG" =~ ^([0-9]+):([0-9]+)$ ]]; then
  ASPECT_W="${BASH_REMATCH[1]}"
  ASPECT_H="${BASH_REMATCH[2]}"
  [[ "$ASPECT_W" -eq 0 || "$ASPECT_H" -eq 0 ]] && error "Aspect ratio values must be non-zero. Got: $ASPECT_ARG"
else
  error "Aspect ratio must be W:H format (e.g. 1:1, 16:9, 4:3). Got: $ASPECT_ARG"
fi

# ── Derive output filename ─────────────────────────────────────────────────────
BASENAME="${INPUT%.*}"
CROPPED="${BASENAME}_cropped_temp.mp4"
OUTPUT="${BASENAME}_180vr.mp4"

# ── Dependency checks ──────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INJECT_SCRIPT="${SCRIPT_DIR}/inject_180_metadata.py"

info "Checking dependencies..."
command -v ffmpeg >/dev/null 2>&1 || error "ffmpeg not found. Install: brew install ffmpeg"
[[ -f "$INJECT_SCRIPT" ]] || error "inject_180_metadata.py not found (expected at ${INJECT_SCRIPT})"

# Prefer uv run (picks up pyproject.toml deps automatically); fall back to python3
if command -v uv >/dev/null 2>&1; then
  PYTHON_RUN="uv run"
else
  command -v python3 >/dev/null 2>&1 || error "python3 not found and uv not installed."
  python3 -c "import spatialmedia" 2>/dev/null \
    || error "spatialmedia not found. Install: pip3 install spatialmedia  (or install uv)"
  PYTHON_RUN="python3"
fi
success "All dependencies found."

# ── Detect source resolution ───────────────────────────────────────────────────
info "Detecting source resolution..."
WIDTH=$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width -of csv=p=0 "$INPUT" | cut -d \, -f 1)
HEIGHT=$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=height -of csv=p=0 "$INPUT" | cut -d \, -f 1)

info "Source: ${WIDTH}×${HEIGHT}"

# Validate 2:1 aspect ratio (equirectangular)
EXPECTED_HEIGHT=$((WIDTH / 2))
if [[ "$HEIGHT" -ne "$EXPECTED_HEIGHT" ]]; then
  warn "Aspect ratio is not 2:1 (${WIDTH}×${HEIGHT}). This may not be equirectangular."
  warn "Make sure you exported as 360° from Insta360 Studio, not as a reframed flat video."
  read -rp "Continue anyway? [y/N] " CONFIRM
  [[ "$CONFIRM" != "y" && "$CONFIRM" != "Y" ]] && exit 1
fi

# ── Calculate crop parameters ──────────────────────────────────────────────────
# Center the 180° window on CENTER_LON degrees.
# x_start = pixel position of the left edge of the crop.
# Uses modular arithmetic so any angle (including wrap-around) is valid.
CROP_W=$((WIDTH / 2))
# Vertical crop centred on the equator, sized to the requested aspect ratio.
# Round CROP_H down to the nearest even number (required by h264).
CROP_H=$(( (CROP_W * ASPECT_H / ASPECT_W / 2) * 2 ))
Y_OFFSET=$(( (HEIGHT - CROP_H) / 2 ))

X_START=$(( (CENTER_LON * WIDTH / 360) - (CROP_W / 2) ))
# Normalise into [0, WIDTH) — bash % can return negative values for negative operands
X_OFFSET=$(( ((X_START % WIDTH) + WIDTH) % WIDTH ))

WRAPS="no"
if [[ $((X_OFFSET + CROP_W)) -gt $WIDTH ]]; then
  WRAPS="yes"
fi

info "Center longitude: ${CENTER_LON}° → x_offset=${X_OFFSET} (wraps seam: ${WRAPS})"
info "Output will be ${CROP_W}×${CROP_H} (${ASPECT_ARG} for 180° VR)"

# ── Step 1: Crop with FFmpeg ───────────────────────────────────────────────────
info "Step 1/2 — Cropping video with FFmpeg..."
info "  This re-encodes video. For large files this may take several minutes."

if [[ "$WRAPS" == "no" ]]; then
  # Simple case: crop doesn't cross the 360°/0° seam
  VF_CROP="crop=${CROP_W}:${CROP_H}:${X_OFFSET}:${Y_OFFSET}"
else
  # Wrap-around: duplicate the frame horizontally (split→hstack), then crop.
  # The normalised X_OFFSET is always in [0, WIDTH), so cropping from the
  # doubled-width frame at X_OFFSET gives seamless wrap-around.
  VF_CROP="split[a][b];[a][b]hstack,crop=${CROP_W}:${CROP_H}:${X_OFFSET}:${Y_OFFSET}"
fi

ffmpeg -i "$INPUT" \
  -filter_complex "$VF_CROP" \
  -c:v libx264 \
  -preset slow \
  -crf 18 \
  -c:a copy \
  -movflags +faststart \
  -map_metadata 0 \
  -y \
  "$CROPPED"

success "Crop complete → $CROPPED"

# ── Step 2: Inject 180° VR metadata ───────────────────────────────────────────
info "Step 2/2 — Injecting 180° spherical metadata..."

$PYTHON_RUN "$INJECT_SCRIPT" \
  "$CROPPED" "$OUTPUT" \
  --full-width  "$WIDTH" \
  --full-height "$HEIGHT" \
  --x-offset    "$X_OFFSET" \
  --top-offset  "$Y_OFFSET"

success "Metadata injection complete → $OUTPUT"

# ── Cleanup ────────────────────────────────────────────────────────────────────
rm -f "$CROPPED"
info "Removed temporary file: $CROPPED"

# ── Summary ────────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Done! Your 180° VR file is ready:${NC}"
echo -e "${GREEN}  → ${OUTPUT}${NC}"
echo -e "${GREEN}════════════════════════════════════════════════${NC}"
echo ""
echo "  Source:  ${INPUT} (${WIDTH}×${HEIGHT} equirectangular 360°)"
echo "  Output:  ${OUTPUT} (${CROP_W}×${CROP_H} equirectangular 180°, ${ASPECT_ARG})"
echo "  Center longitude: ${CENTER_LON}° (x_offset=${X_OFFSET})"
echo ""
echo "  Upload ${OUTPUT} to YouTube."
echo "  YouTube will automatically detect the 180° VR metadata"
echo "  and enable its immersive viewer."
echo ""
echo -e "${YELLOW}  TIP: If the view looks wrong, run preview_crops.sh to pick${NC}"
echo -e "${YELLOW}  a different center longitude, then re-run:${NC}"
echo -e "${YELLOW}  ./preview_crops.sh \"${INPUT}\"${NC}"
echo -e "${YELLOW}  $0 \"${INPUT}\" <degrees>${NC}"
echo ""
