use clap::Parser;
use std::net::IpAddr;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    pub host: IpAddr,

    /// Port to bind to
    #[arg(long, default_value_t = 12206)]
    pub port: u16,

    /// Root directory for video files
    #[arg(long, env = "SPRINTING_BOXES_VIDEO_ROOT")]
    pub video_root: String,

    /// Root directory for output artifacts
    #[arg(long, env = "SPRINTING_BOXES_OUTPUT_ROOT")]
    pub output_root: String,
}

impl Args {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}
