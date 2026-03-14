#!/usr/bin/env python3
"""Inject mono VR180 spherical metadata into an MP4 file."""

import argparse
import sys
from spatialmedia import metadata_utils


def main():
    parser = argparse.ArgumentParser(
        description="Inject mono VR180 spatial metadata into an MP4 file."
    )
    parser.add_argument("input", help="Input MP4 file path")
    parser.add_argument("output", help="Output MP4 file path")
    args = parser.parse_args()

    metadata = metadata_utils.Metadata(
        projection="equirectangular",
        stereo_mode="mono",
    )
    # V1 UUID XML box — required for VLC and older players
    metadata.video = metadata_utils.generate_spherical_xml(
        projection="equirectangular",
        stereo=None,
    )

    messages = []
    metadata_utils.inject_metadata(args.input, args.output, metadata, messages.append)

    for msg in messages:
        print(msg, file=sys.stderr)


if __name__ == "__main__":
    main()
