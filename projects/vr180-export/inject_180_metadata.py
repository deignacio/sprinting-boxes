#!/usr/bin/env python3
"""
inject_180_metadata.py
Injects YouTube-compatible 180° VR spherical metadata into a cropped
equirectangular video using the spatial-media library from GitHub.

Usage:
    python inject_180_metadata.py <input_cropped.mp4> <output_180vr.mp4>

    # If you know the full source dimensions (recommended for accuracy):
    python inject_180_metadata.py input.mp4 output.mp4 --full-width 7680 --full-height 3840 --x-offset 0

Requirements:
    uv add "spatial-media @ git+https://github.com/google/spatial-media.git"
"""

import argparse
import json
import subprocess
import sys


def get_dimensions(file):
    """Return (width, height) of the first video stream via ffprobe."""
    probe = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "v:0",
         "-show_entries", "stream=width,height", "-of", "json", file],
        capture_output=True, text=True, check=True
    )
    stream = json.loads(probe.stdout)["streams"][0]
    return stream["width"], stream["height"]


def inject(input_file, output_file, full_width, full_height, x_offset, top_offset=0):
    try:
        import spatialmedia.metadata_utils as mu
    except ImportError:
        print("ERROR: spatialmedia not found.")
        print('Install with: uv add "spatial-media @ git+https://github.com/google/spatial-media.git"')
        sys.exit(1)

    crop_w, crop_h = get_dimensions(input_file)
    print(f"Detected cropped dimensions: {crop_w}×{crop_h}")

    if full_width is None:
        full_width  = crop_w * 2
        full_height = crop_h
        print(f"Inferred full pano dimensions: {full_width}×{full_height}")

    # Crop string format expected by generate_spherical_xml:
    # "crop_w:crop_h:full_w:full_h:left:top"
    crop_str = f"{crop_w}:{crop_h}:{full_width}:{full_height}:{x_offset}:{top_offset}"

    print(f"\nInjecting metadata:")
    print(f"  Input:      {input_file}")
    print(f"  Output:     {output_file}")
    print(f"  Crop:       {crop_w}×{crop_h}")
    print(f"  Full pano:  {full_width}×{full_height}")
    print(f"  X offset:   {x_offset}")
    print(f"  Top offset: {top_offset}")
    print(f"  Crop str:   {crop_str}")
    print()

    xml = mu.generate_spherical_xml(
        projection="equirectangular",
        stereo=None,   # mono — no StereoMode tag
        crop=crop_str,
    )
    if not xml:
        print("ERROR: generate_spherical_xml returned falsy — check crop parameters.")
        sys.exit(1)

    metadata = mu.Metadata()
    metadata.video = xml   # video must be the XML string, not an object

    mu.inject_metadata(input_file, output_file, metadata, console=print)
    print(f"\n✓ Metadata injected → {output_file}")


def main():
    parser = argparse.ArgumentParser(
        description="Inject 180° VR spherical metadata into a cropped equirectangular video."
    )
    parser.add_argument("input",  help="Cropped input MP4 file")
    parser.add_argument("output", help="Output MP4 file with metadata injected")
    parser.add_argument("--full-width",  type=int, default=None,
                        help="Width of the original full 360° source (e.g. 7680 for 8K). "
                             "Auto-inferred as 2× crop width if omitted.")
    parser.add_argument("--full-height", type=int, default=None,
                        help="Height of the original full 360° source (e.g. 3840 for 8K). "
                             "Auto-inferred as crop height if omitted.")
    parser.add_argument("--x-offset", type=int, default=0,
                        help="Horizontal offset of crop within the full pano. "
                             "0 = front hemisphere (default), half of full-width = back hemisphere.")
    parser.add_argument("--top-offset", type=int, default=0,
                        help="Vertical offset of crop within the full pano. "
                             "0 = top of frame. Use (full_height - crop_height) / 2 to centre vertically.")
    args = parser.parse_args()

    inject(args.input, args.output, args.full_width, args.full_height, args.x_offset, args.top_offset)


if __name__ == "__main__":
    main()
