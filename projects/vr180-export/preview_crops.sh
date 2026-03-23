#!/usr/bin/env bash
# =============================================================================
# preview_crops.sh — Extract 180° crop previews at every 15° longitude
#                    increment so you can pick the right offset visually.
#
# USAGE:
#   ./preview_crops.sh input.mp4
#   ./preview_crops.sh input.mp4 1:1      # default square (full 180°×180°)
#   ./preview_crops.sh input.mp4 16:9     # widescreen aspect ratio
#   ./preview_crops.sh input.mp4 1:1 5.0  # square, extract frame at 5.0 seconds
#
# OUTPUT:
#   input_previews/preview_000deg.jpg  … preview_345deg.jpg
#   input_previews/contact_sheet.jpg   (all 24 crops in a 6×4 grid)
#
# THEN run convert.sh with the winning angle:
#   ./convert.sh input.mp4 90
# =============================================================================

set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }

# ── Args ───────────────────────────────────────────────────────────────────────
INPUT="${1:-}"
ASPECT_ARG="${2:-1:1}" # output aspect ratio W:H — must match what you'll pass to convert.sh
FRAME_TIME="${3:-}"

[[ -z "$INPUT" ]] && error "Usage: $0 <input.mp4> [aspect_ratio] [timestamp_seconds]"
[[ ! -f "$INPUT" ]] && error "File not found: $INPUT"

# Parse aspect ratio W:H
if [[ "$ASPECT_ARG" =~ ^([0-9]+):([0-9]+)$ ]]; then
  ASPECT_W="${BASH_REMATCH[1]}"
  ASPECT_H="${BASH_REMATCH[2]}"
  [[ "$ASPECT_W" -eq 0 || "$ASPECT_H" -eq 0 ]] && error "Aspect ratio values must be non-zero. Got: $ASPECT_ARG"
else
  error "Aspect ratio must be W:H format (e.g. 1:1, 16:9, 4:3). Got: $ASPECT_ARG"
fi

command -v ffmpeg  >/dev/null 2>&1 || error "ffmpeg not found. Install: brew install ffmpeg"
command -v ffprobe >/dev/null 2>&1 || error "ffprobe not found (comes with ffmpeg)."

# ── Output directory ───────────────────────────────────────────────────────────
BASENAME="${INPUT%.*}"
OUT_DIR="${BASENAME}_previews"
mkdir -p "$OUT_DIR"

# ── Detect source resolution ───────────────────────────────────────────────────
info "Detecting source resolution..."
WIDTH=$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width -of csv=p=0 "$INPUT" | cut -d, -f1)
HEIGHT=$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=height -of csv=p=0 "$INPUT" | cut -d, -f1)
info "Source: ${WIDTH}×${HEIGHT}"

CROP_W=$((WIDTH / 2))
CROP_H=$(( (CROP_W * ASPECT_H / ASPECT_W / 2) * 2 ))  # matches convert.sh; even number
Y_OFFSET=$(( (HEIGHT - CROP_H) / 2 ))
PREVIEW_W=1280   # scale down for quick browsing
TILE_W=640
TILE_H=$(( TILE_W * ASPECT_H / ASPECT_W ))

# ── Pick frame time ────────────────────────────────────────────────────────────
if [[ -z "$FRAME_TIME" ]]; then
  DURATION=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$INPUT")
  FRAME_TIME=$(awk "BEGIN { printf \"%.3f\", $DURATION / 2 }")
  info "No timestamp given — using midpoint: ${FRAME_TIME}s"
else
  info "Using timestamp: ${FRAME_TIME}s"
fi

# ── Extract one full-width frame ───────────────────────────────────────────────
FULL_FRAME="${OUT_DIR}/_full_frame.png"
info "Extracting frame..."
ffmpeg -ss "$FRAME_TIME" -i "$INPUT" -frames:v 1 -q:v 1 \
  "$FULL_FRAME" -y -loglevel error
success "Frame extracted → $FULL_FRAME"

# ── Double the frame horizontally for seamless wrap-around crops ───────────────
DOUBLED="${OUT_DIR}/_doubled_frame.png"
ffmpeg -i "$FULL_FRAME" -i "$FULL_FRAME" \
  -filter_complex "[0:v][1:v]hstack" \
  "$DOUBLED" -y -loglevel error

# ── Generate one preview per 15° ──────────────────────────────────────────────
info "Generating 24 preview crops (every 15°)..."
echo ""

PREVIEW_FILES=()
for DEG in $(seq 0 15 345); do
  # Center the 180° window on longitude DEG.
  # x_start of the crop in the original frame (may be negative or wrap past WIDTH).
  X_START=$(( (DEG * WIDTH / 360) - (CROP_W / 2) ))

  # Map into the doubled frame (which is [0, 2*WIDTH)):
  # Shift by +WIDTH so that any original-frame offset in [-WIDTH, WIDTH)
  # lands in [0, 2*WIDTH), then take mod WIDTH to normalise to [0, WIDTH),
  # keeping it within the left copy so the right copy handles the overflow.
  X_IN_DOUBLED=$(( ((X_START % WIDTH) + WIDTH) % WIDTH ))

  OUTFILE="${OUT_DIR}/preview_$(printf '%03d' "$DEG")deg.jpg"
  PREVIEW_FILES+=("$OUTFILE")

  ffmpeg -i "$DOUBLED" \
    -vf "crop=${CROP_W}:${CROP_H}:${X_IN_DOUBLED}:${Y_OFFSET},scale=${PREVIEW_W}:-1" \
    -frames:v 1 -q:v 3 \
    "$OUTFILE" -y -loglevel error

  printf "  %3d°  →  %s\n" "$DEG" "$OUTFILE"
done

echo ""

# ── Contact sheet (6 columns × 4 rows) ────────────────────────────────────────
# Build 4 row images with hstack, then vstack them — more reliable than xstack.
CONTACT_SHEET="${OUT_DIR}/contact_sheet.jpg"
info "Building contact sheet (6×4 grid)..."

ROW_FILES=()
for ROW in 0 1 2 3; do
  ROW_INPUTS=()
  ROW_FILTER=""
  for COL in 0 1 2 3 4 5; do
    IDX=$((ROW * 6 + COL))
    ROW_INPUTS+=(-i "${PREVIEW_FILES[$IDX]}")
    ROW_FILTER+="[$COL:v]scale=${TILE_W}:${TILE_H}[s$COL];"
  done
  ROW_FILTER+="[s0][s1][s2][s3][s4][s5]hstack=inputs=6[row]"
  ROW_FILE="${OUT_DIR}/_row_${ROW}.jpg"
  ROW_FILES+=("$ROW_FILE")

  ffmpeg "${ROW_INPUTS[@]}" \
    -filter_complex "$ROW_FILTER" \
    -map "[row]" -frames:v 1 -q:v 3 \
    "$ROW_FILE" -y -loglevel error
done

ffmpeg -i "${ROW_FILES[0]}" -i "${ROW_FILES[1]}" \
       -i "${ROW_FILES[2]}" -i "${ROW_FILES[3]}" \
  -filter_complex "[0:v][1:v][2:v][3:v]vstack=inputs=4[sheet]" \
  -map "[sheet]" -frames:v 1 -q:v 3 \
  "$CONTACT_SHEET" -y -loglevel error

rm -f "${ROW_FILES[@]}"
success "Contact sheet → $CONTACT_SHEET"

# ── Cleanup temp files ─────────────────────────────────────────────────────────
rm -f "$FULL_FRAME" "$DOUBLED"

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Done! Preview crops are in: ${OUT_DIR}/${NC}"
echo -e "${GREEN}════════════════════════════════════════════════${NC}"
echo ""
echo "  Open the contact sheet for a quick overview:"
echo "  → ${CONTACT_SHEET}"
echo ""
echo "  Then convert with your chosen angle, e.g.:"
echo "  → ./convert.sh \"${INPUT}\" 90 ${ASPECT_ARG}"
echo ""
