use crate::video::processor::VideoSession;
use anyhow::Result;
use opencv::prelude::{MatTraitConst, MatTraitConstManual};
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

    // Create a temporary session to get a reader with 1 sample/second
    let sample_rate = 1.0;
    let mut session = VideoSession::new(video_path, backend, sample_rate)?;

    let mut frame_paths = Vec::new();

    for i in 0..frame_count {
        let timestamp = start_time_secs + (i as f64 * interval_secs);
        // Convert timestamp to unit ID: unit_id / sample_rate = timestamp
        let unit_id = (timestamp * sample_rate) as usize;

        if session.reader.seek_to_frame(unit_id).is_ok() {
            if let Ok(mat) = session.reader.read_frame() {
                let filename = format!("frame_{:03}.jpg", i + 1);
                let output_path = output_dir.join(&filename);

                // Convert Mat BGR bytes to RGB and save as JPEG
                if let Ok(data) = mat.data_bytes() {
                    let width = mat.cols() as u32;
                    let height = mat.rows() as u32;
                    // BGR to RGB conversion: swap channels
                    let rgb: Vec<u8> = data
                        .chunks(3)
                        .flat_map(|p| [p[2], p[1], p[0]])
                        .collect::<Vec<_>>();
                    image::save_buffer(
                        &output_path,
                        &rgb,
                        width,
                        height,
                        image::ColorType::Rgb8,
                    )?;
                    frame_paths.push(output_path);
                }
            } else {
                eprintln!(
                    "  Warning: Failed to read frame after seeking to {}s (unit_id {})",
                    timestamp, unit_id
                );
            }
        } else {
            eprintln!(
                "  Warning: Failed to seek to {}s (unit_id {})",
                timestamp, unit_id
            );
        }
    }

    Ok(frame_paths)
}
