pub mod calibration;
pub mod ffmpeg_reader;
pub mod opencv_reader;
pub mod processor;

use anyhow::Result;
use opencv::core::Mat;

pub trait VideoReader: Send {
    fn frame_count(&self) -> Result<usize>;
    fn read_unit(&mut self, unit_id: usize) -> Result<Mat>;
    fn read_frame(&mut self) -> Result<Mat>;
    fn source_fps(&self) -> Result<f64>;
    fn seek_to_frame(&mut self, frame_num: usize) -> Result<()>;
}

/// Map a sampled unit index to its absolute raw frame index in the video.
/// This uses floating-point math to ensure zero cumulative drift.
pub fn unit_to_frame(unit_id: usize, source_fps: f64, sample_rate: f64) -> usize {
    (unit_id as f64 * source_fps / sample_rate).round() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unit_to_frame_mapping() {
        // 30fps video, 1fps sampling
        assert_eq!(unit_to_frame(0, 30.0, 1.0), 0);
        assert_eq!(unit_to_frame(1, 30.0, 1.0), 30);
        assert_eq!(unit_to_frame(10, 30.0, 1.0), 300);

        // 29.97fps video, 1fps sampling (common NTSC)
        assert_eq!(unit_to_frame(0, 29.97, 1.0), 0);
        assert_eq!(unit_to_frame(1, 29.97, 1.0), 30); // 29.97 rounds to 30
        assert_eq!(unit_to_frame(10, 29.97, 1.0), 300); // 299.7 rounds to 300
        assert_eq!(unit_to_frame(100, 29.97, 1.0), 2997);

        // 29.0fps video, 1fps sampling (the user's ghost case)
        assert_eq!(unit_to_frame(0, 29.0, 1.0), 0);
        assert_eq!(unit_to_frame(1, 29.0, 1.0), 29);
        assert_eq!(unit_to_frame(10, 29.0, 1.0), 290);
    }
}
