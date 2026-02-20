use super::VideoReader;
use anyhow::{anyhow, Result};
use opencv::{
    prelude::*,
    videoio::{
        VideoCapture, CAP_AVFOUNDATION, CAP_PROP_FPS, CAP_PROP_FRAME_COUNT,
        CAP_PROP_HW_ACCELERATION, CAP_PROP_POS_FRAMES, CAP_PROP_POS_MSEC, VIDEO_ACCELERATION_ANY,
    },
};

pub struct OpencvReader {
    capture: VideoCapture,
    source_fps: f64,
    sample_rate: f64,
    total_frames: usize,
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
        let raw_count = capture.get(CAP_PROP_FRAME_COUNT)? as usize;
        let duration_secs = if fps > 0.0 {
            raw_count as f64 / fps
        } else {
            0.0
        };

        tracing::info!(
            "OpencvReader: opened {}, duration={:.2}s, fps={:.2}, stream_frames={}",
            path,
            duration_secs,
            fps,
            raw_count
        );

        Ok(Self {
            capture,
            source_fps: fps,
            sample_rate,
            total_frames: raw_count,
        })
    }
}

impl VideoReader for OpencvReader {
    fn frame_count(&self) -> Result<usize> {
        let units =
            (self.total_frames as f64 * self.sample_rate / self.source_fps).floor() as usize;
        Ok(units.max(1))
    }

    fn source_fps(&self) -> Result<f64> {
        Ok(self.source_fps)
    }

    fn seek_to_frame(&mut self, frame_num: usize) -> Result<()> {
        self.capture.set(CAP_PROP_POS_FRAMES, frame_num as f64)?;
        Ok(())
    }

    fn read_frame(&mut self) -> Result<Mat> {
        let mut frame = Mat::default();
        let success = self.capture.read(&mut frame)?;
        if !success || frame.empty() {
            return Err(anyhow!("Failed to read frame"));
        }

        Ok(frame)
    }

    fn read_unit(&mut self, unit_id: usize) -> Result<Mat> {
        let target_msec = super::unit_to_msec(unit_id, self.sample_rate);
        let current_msec = self.capture.get(CAP_PROP_POS_MSEC)?;

        if (target_msec - current_msec).abs() > 1.0 {
            // Seek if more than 1ms away
            self.capture.set(CAP_PROP_POS_MSEC, target_msec)?;
        }

        self.read_frame()
    }
}
