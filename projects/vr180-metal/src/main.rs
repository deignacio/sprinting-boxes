mod decode;
mod encode;
mod gpu;
mod mux;
mod pipeline;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "vr180-metal",
    about = "GPU-accelerated equirectangular → mono fisheye converter (Metal + VideoToolbox)"
)]
struct Args {
    /// Input equirectangular MP4
    #[arg(long, short)]
    input: PathBuf,

    /// Output fisheye MP4 (no VR metadata — run inject_metadata.py afterwards)
    #[arg(long, short)]
    output: PathBuf,

    /// Horizontal rotation in degrees
    #[arg(long, default_value_t = 0.0)]
    yaw: f32,

    /// Vertical rotation in degrees
    #[arg(long, default_value_t = 0.0)]
    pitch: f32,

    /// Roll rotation in degrees
    #[arg(long, default_value_t = 0.0)]
    roll: f32,
}

fn main() -> Result<()> {
    let args = Args::parse();
    pipeline::run(&args.input, &args.output, args.yaw, args.pitch, args.roll)
}
