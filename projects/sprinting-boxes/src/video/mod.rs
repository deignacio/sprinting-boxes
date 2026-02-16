pub mod calibration;
pub mod ffmpeg_reader;
pub mod opencv_reader;
pub mod processor;

use anyhow::Result;
use opencv::core::Mat;

pub trait VideoReader: Send {
    fn frame_count(&self) -> Result<usize>;
    fn next_frame(&mut self) -> Result<Mat>;
    fn source_fps(&self) -> Result<f64>;
    fn seek_to_frame(&mut self, frame_num: usize) -> Result<()>;
}
