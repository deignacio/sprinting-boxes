use super::VideoReader;
use anyhow::{anyhow, Result};
use opencv::{
    prelude::*,
    videoio::{
        VideoCapture, CAP_AVFOUNDATION, CAP_PROP_FPS, CAP_PROP_FRAME_COUNT,
        CAP_PROP_HW_ACCELERATION, CAP_PROP_POS_FRAMES, VIDEO_ACCELERATION_ANY,
    },
};

pub struct OpencvReader {
    capture: VideoCapture,
    _source_fps: f64,
    _skip_count: usize,
}

impl OpencvReader {
    pub fn new(path: &str, sample_rate: f64) -> Result<Self> {
        let mut capture = VideoCapture::from_file(path, CAP_AVFOUNDATION)?;
        if !capture.is_opened()? {
            return Err(anyhow!("Failed to open video file: {}", path));
        }

        // Try to enable hardware acceleration (VideoToolbox on macOS, VA-API on Linux, etc.)
        let hw_result = capture.set(CAP_PROP_HW_ACCELERATION, VIDEO_ACCELERATION_ANY as f64);
        if let Ok(success) = hw_result {
            if success {
                println!("Hardware acceleration enabled.");
            } else {
                println!("Hardware acceleration not available or failed to enable.");
            }
        }

        let mut fps = capture.get(CAP_PROP_FPS)?;
        if fps <= 0.0 {
            tracing::warn!("OpencvReader: Failed to get FPS from metadata, falling back to 30.0");
            fps = 30.0;
        }
        let skip_count = (fps / sample_rate).round() as usize;

        Ok(Self {
            capture,
            _source_fps: fps,
            _skip_count: skip_count.max(1),
        })
    }
}

impl VideoReader for OpencvReader {
    fn frame_count(&self) -> Result<usize> {
        let raw_count = self.capture.get(CAP_PROP_FRAME_COUNT)? as usize;
        if self._skip_count > 0 {
            // Number of times we call next_frame()
            // Each call reads one frame and then grabs skip_count - 1 frames
            Ok(raw_count.div_ceil(self._skip_count))
        } else {
            Ok(raw_count)
        }
    }

    fn source_fps(&self) -> Result<f64> {
        Ok(self._source_fps)
    }

    fn seek_to_frame(&mut self, frame_num: usize) -> Result<()> {
        self.capture.set(CAP_PROP_POS_FRAMES, frame_num as f64)?;
        Ok(())
    }

    fn next_frame(&mut self) -> Result<Mat> {
        let mut frame = Mat::default();
        let success = self.capture.read(&mut frame)?;
        if !success || frame.empty() {
            return Err(anyhow!("Failed to read frame"));
        }

        // The read() already advanced by 1.
        // If we want to skip N frames total between samples, we grab N-1 more.
        if self._skip_count > 1 {
            for _ in 0..(self._skip_count - 1) {
                if !self.capture.grab()? {
                    break;
                }
            }
        }

        Ok(frame)
    }
}
