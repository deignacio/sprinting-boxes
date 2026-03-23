#include <metal_stdlib>
using namespace metal;

// ── Passthrough ───────────────────────────────────────────────────────────────
// Copies each source pixel into the destination at the same position.
// Input and output textures must be the same dimensions.
kernel void k_passthrough(
    texture2d<float, access::read>  src [[texture(0)]],
    texture2d<float, access::write> dst [[texture(1)]],
    uint2 gid [[thread_position_in_grid]]
) {
    if (gid.x >= dst.get_width() || gid.y >= dst.get_height()) return;
    dst.write(src.read(gid), gid);
}

// ── Crop with horizontal wrap-around ─────────────────────────────────────────
// For each destination pixel (gid), reads from:
//   src_x = (gid.x + x_offset) % src_width   ← modulo handles 360°/0° seam wrap
//   src_y =  gid.y + y_offset
//
// The destination texture is (crop_w × crop_h); the source is the full frame.
struct CropParams {
    uint x_offset;   // leftmost source column mapped to dst column 0
    uint y_offset;   // topmost  source row    mapped to dst row    0
    uint src_width;  // full source frame width (for modulo wrap)
};

kernel void k_crop(
    texture2d<float, access::read>  src    [[texture(0)]],
    texture2d<float, access::write> dst    [[texture(1)]],
    constant CropParams&            params [[buffer(0)]],
    uint2 gid [[thread_position_in_grid]]
) {
    if (gid.x >= dst.get_width() || gid.y >= dst.get_height()) return;
    uint src_x = (gid.x + params.x_offset) % params.src_width;
    uint src_y =  gid.y + params.y_offset;
    dst.write(src.read(uint2(src_x, src_y)), gid);
}
