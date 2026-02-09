#![allow(dead_code)]
use crate::video::{opencv_reader::OpencvReader, VideoReader};
use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use opencv::core::Mat;
use std::time::{Duration, Instant};

pub struct ProcessingStats {
    pub processed_frames: usize,
    pub duration: Duration,
}

/// A trait for handling video frames. This separates the "how to process"
/// from the "how to read and orchestrate" logic.
pub trait FrameProcessor {
    fn process(&mut self, frame: Mat) -> Result<()>;
}

/// Blanket implementation so any closure with the right signature
/// automatically implements FrameProcessor.
impl<F> FrameProcessor for F
where
    F: FnMut(Mat) -> Result<()>,
{
    fn process(&mut self, frame: Mat) -> Result<()> {
        self(frame)
    }
}

pub struct VideoSession {
    pub reader: Box<dyn VideoReader>,
    pub pb: ProgressBar,
    pub start_time: Instant,
    pub processed_frames: usize,
}

impl VideoSession {
    pub fn new(video_path: &str, backend: &str, sample_rate: f64) -> Result<Self> {
        let reader: Box<dyn VideoReader> = match backend {
            "opencv" => Box::new(OpencvReader::new(video_path, sample_rate)?),
            _ => {
                return Err(anyhow::anyhow!(
                    "Unsupported or disabled backend: {}",
                    backend
                ))
            }
        };

        let total_frames = reader.frame_count()?;
        let source_fps = reader.source_fps()?;

        let sampled_frames = (total_frames as f64 / source_fps * sample_rate) as usize;

        let pb = ProgressBar::new(sampled_frames as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({per_sec:.1.yellow} fps, {eta})")?
                .progress_chars("#>-"),
        );

        Ok(Self {
            reader,
            pb,
            start_time: Instant::now(),
            processed_frames: 0,
        })
    }
}

pub fn process_video<P>(
    video_path: &str,
    backend: &str,
    sample_rate: f64,
    mut processor: P,
) -> Result<ProcessingStats>
where
    P: FrameProcessor,
{
    let mut session = VideoSession::new(video_path, backend, sample_rate)?;

    while let Ok(frame) = session.reader.next_frame() {
        processor.process(frame)?;
        session.processed_frames += 1;
        session.pb.inc(1);
    }

    session.pb.finish_with_message("Done");

    Ok(ProcessingStats {
        processed_frames: session.processed_frames,
        duration: session.start_time.elapsed(),
    })
}
