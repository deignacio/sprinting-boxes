mod decode;
mod encode;
mod gpu;
mod mux;
mod pipeline;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "zero-copy-utils",
    about = "GPU-accelerated zero-copy video processing (Metal + VideoToolbox)"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Decode every frame and re-encode it unchanged.
    /// Use this as a sanity check that the full decode → GPU → encode pipeline works.
    Passthrough {
        /// Input HEVC MP4
        #[arg(long, short)]
        input: PathBuf,
        /// Output MP4
        #[arg(long, short)]
        output: PathBuf,
    },

    /// Crop a rectangle from the source frame using a zero-copy Metal kernel.
    ///
    /// Horizontal wrap-around is handled automatically: an x_offset that would
    /// extend past the right edge wraps back around to the left, which is
    /// correct for 360° equirectangular sources centered at an arbitrary longitude.
    Crop {
        /// Input HEVC MP4
        #[arg(long, short)]
        input: PathBuf,
        /// Output MP4
        #[arg(long, short)]
        output: PathBuf,
        /// Leftmost source column mapped to output column 0 (pixels)
        #[arg(long)]
        x_offset: u32,
        /// Topmost source row mapped to output row 0 (pixels)
        #[arg(long, default_value_t = 0)]
        y_offset: u32,
        /// Output width in pixels
        #[arg(long)]
        crop_w: u32,
        /// Output height in pixels
        #[arg(long)]
        crop_h: u32,
    },

    /// Crop a spherical region from an equirectangular source, specified in
    /// degrees.  Pixel offsets are derived automatically from the source
    /// resolution.  Longitude wrap-around at the ±180° seam is handled by
    /// the Metal shader.
    ///
    /// Example — front hemisphere VR180 from an 8K 360° source:
    ///   spherical-crop --input 360.mp4 --output 180.mp4
    ///
    /// Degrees match the values shown by preview_crops.sh / crop.sh:
    ///   0   = left edge of equirectangular frame (Insta360 X5 front)
    ///   180 = center of frame (back hemisphere)
    SphericalCrop {
        /// Input HEVC MP4
        #[arg(long, short)]
        input: PathBuf,
        /// Output MP4
        #[arg(long, short)]
        output: PathBuf,
        /// Longitude of the crop centre (0–360, same scale as preview_crops.sh).
        /// 0 = left edge / Insta360 X5 front hemisphere.
        #[arg(long, default_value_t = 0.0)]
        lon_center: f64,
        /// Shift the crop vertically: positive moves up, negative down (degrees).
        /// 0 = centred on equator (default).
        #[arg(long, default_value_t = 0.0)]
        lat_center: f64,
        /// Horizontal field of view in degrees
        #[arg(long, default_value_t = 180.0)]
        fov_h: f64,
        /// Vertical field of view in degrees
        #[arg(long, default_value_t = 180.0)]
        fov_v: f64,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Passthrough { input, output } => {
            pipeline::run(&input, &output, pipeline::Operation::Passthrough)
        }
        Command::Crop { input, output, x_offset, y_offset, crop_w, crop_h } => {
            pipeline::run(
                &input,
                &output,
                pipeline::Operation::Crop { x_offset, y_offset, crop_w, crop_h },
            )
        }
        Command::SphericalCrop { input, output, lon_center, lat_center, fov_h, fov_v } => {
            pipeline::run(
                &input,
                &output,
                pipeline::Operation::SphericalCrop { lon_center, lat_center, fov_h, fov_v },
            )
        }
    }
}
