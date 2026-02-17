use crate::video::processor::VideoSession;
use anyhow::Result;
use opencv::imgcodecs;
use std::path::{Path, PathBuf};

pub fn extract_calibration_frames(
    video_path: &str,
    backend: &str,
    output_dir: &Path,
    start_time_secs: f64,
    frame_count: usize,
    interval_secs: f64,
) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(output_dir)?;

    // Create a temporary session to get a reader
    let mut session = VideoSession::new(video_path, backend, 1.0)?;
    let source_fps = session.reader.source_fps().unwrap_or(30.0);

    let mut frame_paths = Vec::new();

    for i in 0..frame_count {
        let timestamp = start_time_secs + (i as f64 * interval_secs);
        let frame_index = (timestamp * source_fps) as usize;

        if session.reader.seek_to_frame(frame_index).is_ok() {
            if let Ok(mat) = session.reader.read_frame() {
                let filename = format!("frame_{:03}.jpg", i + 1);
                let output_path = output_dir.join(&filename);

                let params = opencv::core::Vector::<i32>::new();
                imgcodecs::imwrite(output_path.to_str().unwrap(), &mat, &params)?;

                frame_paths.push(output_path);
            } else {
                eprintln!(
                    "  Warning: Failed to read frame after seeking to {}s (frame {})",
                    timestamp, frame_index
                );
            }
        } else {
            eprintln!(
                "  Warning: Failed to seek to {}s (frame {})",
                timestamp, frame_index
            );
        }
    }

    Ok(frame_paths)
}
