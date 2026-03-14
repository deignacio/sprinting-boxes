#include <metal_stdlib>
using namespace metal;

// Equirectangular → equidistant fisheye reprojection kernel.
//
// For each output pixel in the fisheye image, we compute the corresponding
// source pixel in the equirectangular image and sample it.
//
// Fisheye model: equidistant (r = f * theta), where r is the normalised
// distance from the image centre (0..1 at the 180° circle edge) and theta
// is the angle from the optical axis (0..π/2 for a 180° FOV lens).
//
// Input:  src  — equirectangular texture  (src_w × src_h, e.g. 8640×4320)
// Output: dst  — square fisheye texture   (out_size × out_size, e.g. 4320×4320)
// Buffer: rotation — row-major 3×3 float rotation matrix (yaw / pitch / roll)

kernel void reproject(
    texture2d<float, access::sample> src [[ texture(0) ]],
    texture2d<float, access::write>  dst [[ texture(1) ]],
    constant float3x3&         rotation  [[ buffer(0)  ]],
    uint2 gid                            [[ thread_position_in_grid ]]
) {
    const uint out_w = dst.get_width();
    const uint out_h = dst.get_height();

    if (gid.x >= out_w || gid.y >= out_h) {
        return;
    }

    // Normalise pixel to [-1, 1] centred on image centre.
    const float cx = float(out_w) * 0.5f;
    const float cy = float(out_h) * 0.5f;
    const float dx = (float(gid.x) - cx) / cx;   // [-1, 1]
    const float dy = (float(gid.y) - cy) / cy;   // [-1, 1]

    const float r = sqrt(dx * dx + dy * dy);      // 0 at centre, 1 at 180° edge

    // Pixels outside the unit circle are outside the 180° fisheye FOV.
    if (r > 1.0f) {
        dst.write(float4(0.0f, 0.0f, 0.0f, 1.0f), gid);
        return;
    }

    // Equidistant fisheye: theta = r * (π/2)
    const float theta = r * (M_PI_2_F);           // polar angle [0, π/2]
    const float phi   = atan2(dy, dx);            // azimuth [-π, π]

    // 3-D unit direction in camera space (forward = +Z).
    const float sin_theta = sin(theta);
    float3 dir = float3(
        sin_theta * cos(phi),
        sin_theta * sin(phi),
        cos(theta)
    );

    // Apply rotation (yaw / pitch / roll).
    // rotation is row-major: result = rotation * dir
    float3 rd = rotation * dir;

    // Convert rotated direction to equirectangular (lon, lat).
    const float lon = atan2(rd.x, rd.z);                      // [-π, π]
    const float lat = atan2(rd.y, sqrt(rd.x * rd.x + rd.z * rd.z)); // [-π/2, π/2]

    // Map to normalised texture coordinates [0, 1].
    const float u = lon / (2.0f * M_PI_F) + 0.5f;
    const float v = 0.5f - lat / M_PI_F;

    constexpr sampler s(coord::normalized,
                        address::repeat,
                        filter::linear);
    const float4 colour = src.sample(s, float2(u, v));
    dst.write(colour, gid);
}
