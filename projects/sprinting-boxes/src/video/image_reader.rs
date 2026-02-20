use crate::video::VideoReader;
use anyhow::Result;
use opencv::{
    core::{Mat, MatTraitConst},
    imgcodecs,
};
use std::path::{Path, PathBuf};

/// A "video reader" that actually streams pre-extracted image crops from disk.
/// This is used exclusively by the Field Pipeline to bypass video decoding.
pub struct ImageDiskReader {
    frames_dir: PathBuf,
    total_frames: usize,
    sample_rate: f64,
}

impl ImageDiskReader {
    pub fn new<P: AsRef<Path>>(frames_dir: P, sample_rate: f64) -> Result<Self> {
        let dir = frames_dir.as_ref().to_path_buf();

        if !dir.exists() {
            anyhow::bail!("Frames directory does not exist: {:?}", dir);
        }

        // Count how many overview frames exist to determine total_frames
        let mut count = 0;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("frame_") && name.ends_with("_overview.jpg") {
                        count += 1;
                    }
                }
            }
        }

        Ok(Self {
            frames_dir: dir,
            total_frames: count,
            sample_rate,
        })
    }
}

impl VideoReader for ImageDiskReader {
    fn frame_count(&self) -> Result<usize> {
        Ok(self.total_frames)
    }

    fn read_unit(&mut self, unit_id: usize) -> Result<Mat> {
        // Find the frame file for this unit.
        // The files are named frame_{unit:06}_overview.jpg
        let filename = format!("frame_{:06}_overview.jpg", unit_id);
        let path = self.frames_dir.join(&filename);

        if !path.exists() {
            return Err(anyhow::anyhow!("Frame file not found: {:?}", path));
        }

        let path_str = path.to_str().unwrap();
        let mat = imgcodecs::imread(path_str, imgcodecs::IMREAD_COLOR)?;

        if mat.empty() {
            return Err(anyhow::anyhow!("Failed to decode image: {:?}", path));
        }

        Ok(mat)
    }

    fn read_frame(&mut self) -> Result<Mat> {
        Err(anyhow::anyhow!(
            "read_frame is not supported by ImageDiskReader"
        ))
    }

    fn source_fps(&self) -> Result<f64> {
        // Return a dummy value since we're not reading a real video
        Ok(self.sample_rate)
    }

    fn seek_to_frame(&mut self, _frame_num: usize) -> Result<()> {
        Err(anyhow::anyhow!("seek is not supported by ImageDiskReader"))
    }
}
