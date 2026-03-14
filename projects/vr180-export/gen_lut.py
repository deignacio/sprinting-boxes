#!/usr/bin/env python3
"""
Generate xmap/ymap PGM lookup tables for equirectangular → equidistant fisheye reprojection.

The remap filter uses these to look up, for each output pixel, which source pixel to sample.
16-bit grayscale PGM: values encode source pixel coordinates scaled to [0, 65535].
"""

import argparse
import math
import struct
import sys
import numpy as np


def rotation_matrix(yaw_deg: float, pitch_deg: float, roll_deg: float) -> np.ndarray:
    """Build a ZYX rotation matrix from yaw/pitch/roll in degrees."""
    yaw   = math.radians(yaw_deg)
    pitch = math.radians(pitch_deg)
    roll  = math.radians(roll_deg)

    cy, sy = math.cos(yaw),   math.sin(yaw)
    cp, sp = math.cos(pitch), math.sin(pitch)
    cr, sr = math.cos(roll),  math.sin(roll)

    Rz = np.array([[cy, -sy, 0], [sy,  cy, 0], [0, 0, 1]], dtype=np.float64)
    Ry = np.array([[cp,  0, sp], [0,   1,  0], [-sp, 0, cp]], dtype=np.float64)
    Rx = np.array([[1,   0,  0], [0,  cr, -sr], [0, sr,  cr]], dtype=np.float64)

    return Rz @ Ry @ Rx


def generate_lut(
    src_w: int, src_h: int,
    out_size: int,
    yaw: float, pitch: float, roll: float,
) -> tuple[np.ndarray, np.ndarray]:
    """
    For each output pixel in a square [out_size x out_size] fisheye image,
    compute the corresponding source pixel in a [src_w x src_h] equirectangular image.

    Returns (xmap, ymap) as float32 arrays of shape (out_size, out_size),
    where values are source pixel coordinates.
    """
    R = rotation_matrix(yaw, pitch, roll)

    # Output pixel grid, centered at (cx, cy)
    cx = cy = (out_size - 1) / 2.0
    radius = out_size / 2.0

    oy, ox = np.mgrid[0:out_size, 0:out_size]
    dx = (ox - cx) / radius   # normalized [-1, 1]
    dy = (oy - cy) / radius

    r = np.sqrt(dx**2 + dy**2)  # normalized radius in [0, 1] at edge

    # Equidistant fisheye: r = theta / (pi/2), so theta = r * pi/2
    # Pixels outside the unit circle are outside the 180° FOV — map to black
    valid = r <= 1.0

    theta = np.where(valid, r * (math.pi / 2.0), 0.0)  # polar angle from forward axis
    phi   = np.arctan2(dy, dx)                          # azimuth

    # Spherical direction in camera space
    sin_theta = np.sin(theta)
    vx = sin_theta * np.cos(phi)
    vy = sin_theta * np.sin(phi)
    vz = np.cos(theta)

    # Stack into (3, out_size*out_size), rotate, unstack
    vecs = np.stack([vx.ravel(), vy.ravel(), vz.ravel()], axis=0)  # (3, N)
    rvecs = R @ vecs  # (3, N)

    rx, ry, rz = rvecs[0], rvecs[1], rvecs[2]

    # Back to spherical lon/lat
    lon = np.arctan2(rx, rz)   # [-pi, pi]
    lat = np.arctan2(ry, np.sqrt(rx**2 + rz**2))  # [-pi/2, pi/2]

    # Map to equirectangular source pixel
    src_x = (lon / (2 * math.pi) + 0.5) * src_w
    src_y = (0.5 - lat / math.pi) * src_h

    src_x = np.clip(src_x, 0, src_w - 1).reshape(out_size, out_size)
    src_y = np.clip(src_y, 0, src_h - 1).reshape(out_size, out_size)

    # Outside fisheye circle: map to (0, 0) — remap fill=black handles the border
    src_x = np.where(valid, src_x, 0.0).astype(np.float32)
    src_y = np.where(valid, src_y, 0.0).astype(np.float32)

    return src_x, src_y


def write_pgm16(path: str, data: np.ndarray, max_val: int) -> None:
    """Write a 16-bit binary PGM file."""
    h, w = data.shape
    header = f"P5\n{w} {h}\n{max_val}\n".encode()
    scaled = np.clip(data, 0, max_val).astype(np.uint16)
    with open(path, "wb") as f:
        f.write(header)
        # PGM 16-bit is big-endian
        f.write(scaled.byteswap().tobytes())


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate xmap/ymap PGMs for equirectangular→fisheye remap."
    )
    parser.add_argument("--src-w",   type=int,   required=True, help="Source video width")
    parser.add_argument("--src-h",   type=int,   required=True, help="Source video height")
    parser.add_argument("--out-size",type=int,   required=True, help="Output square size (pixels)")
    parser.add_argument("--yaw",     type=float, default=0.0,   help="Yaw rotation degrees")
    parser.add_argument("--pitch",   type=float, default=0.0,   help="Pitch rotation degrees")
    parser.add_argument("--roll",    type=float, default=0.0,   help="Roll rotation degrees")
    parser.add_argument("--xmap",    required=True, help="Output xmap PGM path")
    parser.add_argument("--ymap",    required=True, help="Output ymap PGM path")
    args = parser.parse_args()

    print(f"Generating LUT {args.src_w}x{args.src_h} → {args.out_size}x{args.out_size} "
          f"(yaw={args.yaw} pitch={args.pitch} roll={args.roll})", file=sys.stderr)

    xmap, ymap = generate_lut(
        args.src_w, args.src_h, args.out_size,
        args.yaw, args.pitch, args.roll,
    )

    write_pgm16(args.xmap, xmap, args.src_w - 1)
    write_pgm16(args.ymap, ymap, args.src_h - 1)

    print(f"Written: {args.xmap}", file=sys.stderr)
    print(f"Written: {args.ymap}", file=sys.stderr)


if __name__ == "__main__":
    main()
