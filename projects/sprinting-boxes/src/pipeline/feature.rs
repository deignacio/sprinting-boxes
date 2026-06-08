use crate::config::DetectorConfig;
use crate::scoring::{calculate_deltas, calculate_frame_metrics, FrameHistory};
use ultimate_event_detection::{
    detect_pull_side, CliffDetector, CliffDetectorConfig, EndZoneOccupancy, PullSide,
};
use anyhow::Result;
use crossbeam::channel::{Receiver, Sender};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use crate::pipeline::types::{DetectedFrame, ProcessingState};

// Re-export FeatureConfig for backward compatibility with callers using crate::pipeline::feature::FeatureConfig
pub use crate::scoring::FeatureConfig;

/// Feature worker: calculates normalized counts, pre-point scores, and detects cliffs.
///
/// This worker processes detected frames and:
/// 1. Calculates normalized player counts per endzone (left/right/field)
/// 2. Computes pre-point scores using heuristics
/// 3. Detects point-start transitions (cliffs) using smoothing and plateau detection
/// 4. Applies side heuristics to determine which team pulled
/// 5. Writes incremental CSV exports (features.csv and points.csv)
///
/// The worker uses lookahead/lookback buffering to ensure accurate cliff detection
/// and heuristic analysis before finalizing each frame.
pub fn feature_worker(
    rx: Receiver<DetectedFrame>,
    tx_f: Sender<DetectedFrame>,
    config: FeatureConfig,
    state: Arc<ProcessingState>,
) -> Result<()> {
    use std::io::Write;

    let features_path = config.output_dir.join("features.csv");
    let points_path = config.output_dir.join("points.csv");
    tracing::info!(
        "Feature worker started. output_dir: {:?}",
        config.output_dir
    );

    let mut features_csv = std::fs::File::create(&features_path)?;
    writeln!(
        features_csv,
        "frame_index,left_count,right_count,field_count,pre_point_score,is_cliff,com_x,com_y,distribution_std_dev,com_delta_x,com_delta_y,std_dev_delta"
    )?;

    let mut points_csv = std::fs::File::create(&points_path)?;
    writeln!(
        points_csv,
        "frame_index,is_cliff,left_side_emptied_first,right_side_emptied_first"
    )?;

    let mut input_buffer: BTreeMap<usize, DetectedFrame> = BTreeMap::new();
    let mut next_input_id = 0;
    let mut lookahead_buffer: Vec<DetectedFrame> = Vec::new();
    let mut history_buffer: Vec<FrameHistory> = Vec::new();

    // Load detector config from file (falls back to defaults if file missing)
    let detector_config = DetectorConfig::from_file("detector.config.yaml");
    let cliff_config = CliffDetectorConfig::from(detector_config);
    let mut cliff_state = CliffDetector::new(cliff_config);

    for frame in rx {
        let start_inst = Instant::now();

        if !state.is_active.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        input_buffer.insert(frame.id, frame);

        while let Some(mut current_frame) = input_buffer.remove(&next_input_id) {
            let (_left_raw, _right_raw, _field_raw, _pre_point_score, _com_x_opt, _com_y_opt) =
                calculate_frame_metrics(&mut current_frame, &config);

            // Calculate deltas
            calculate_deltas(&mut current_frame, history_buffer.last());

            // Record history
            history_buffer.push(FrameHistory {
                left_count: current_frame.left_count,
                right_count: current_frame.right_count,
                com_x: current_frame.com_x,
                com_y: current_frame.com_y,
                std_dev: current_frame.std_dev,
            });

            // Run cliff detector
            let cliff_results = cliff_state.push(current_frame.id, current_frame.pre_point_score);

            // Add to lookahead buffer
            lookahead_buffer.push(current_frame);

            // Back-fill cliff status
            for (cliff_frame_idx, is_cliff) in cliff_results {
                if is_cliff {
                    if let Some(frame) = lookahead_buffer
                        .iter_mut()
                        .find(|f| f.id == cliff_frame_idx)
                    {
                        frame.is_cliff = true;
                    }
                }
            }

            // Process buffer if we have enough lookahead
            if lookahead_buffer.len() > config.lookahead_frames {
                let mut frame = lookahead_buffer.remove(0);

                // Apply heuristics if cliff
                if frame.is_cliff {
                    let start_idx = frame.id.saturating_sub(config.lookback_frames);
                    let end_idx = (frame.id + config.lookahead_frames).min(history_buffer.len().saturating_sub(1));

                    let window: Vec<(usize, EndZoneOccupancy)> = (start_idx..=end_idx)
                        .map(|i| {
                            let h = &history_buffer[i];
                            (i, EndZoneOccupancy { left: h.left_count, right: h.right_count, field: 0.0 })
                        })
                        .collect();

                    match detect_pull_side(&window, 2) {
                        PullSide::Left => frame.left_emptied_first = true,
                        PullSide::Right => frame.right_emptied_first = true,
                        PullSide::Tie => {
                            frame.left_emptied_first = true;
                            frame.right_emptied_first = true;
                        }
                        PullSide::Unknown => frame.maybe_false_positive = true,
                    }
                }

                // Write to CSV files
                writeln!(
                    features_csv,
                    "{},{:.5},{:.5},{:.3},{:.3},{},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5}",
                    frame.id,
                    frame.left_count,
                    frame.right_count,
                    frame.field_count,
                    frame.pre_point_score,
                    if frame.is_cliff { 1 } else { 0 },
                    frame.com_x.unwrap_or(-1.0),
                    frame.com_y.unwrap_or(-1.0),
                    frame.std_dev.unwrap_or(-1.0),
                    frame.com_delta_x.unwrap_or(0.0),
                    frame.com_delta_y.unwrap_or(0.0),
                    frame.std_dev_delta.unwrap_or(0.0),
                )?;

                if frame.is_cliff {
                    writeln!(
                        points_csv,
                        "{},{},{},{}",
                        frame.id,
                        if frame.is_cliff { 1 } else { 0 },
                        if frame.left_emptied_first { 1 } else { 0 },
                        if frame.right_emptied_first { 1 } else { 0 }
                    )?;
                }

                let duration_ms = start_inst.elapsed().as_secs_f64() * 1000.0;
                state.update_stage("feature", 1, duration_ms);

                if tx_f.send(frame.clone()).is_err() {
                    tracing::error!("Feature worker: failed to send frame to finalize");
                    break;
                }
            }
            next_input_id += 1;
        }
    }

    tracing::info!("Feature worker: input channel closed. Flushing buffers...");

    // 1. Flush reordering buffer (input_buffer)
    // If there were gaps, we just skip them and process what we have.
    while !input_buffer.is_empty() {
        if let Some(mut current_frame) = input_buffer.remove(&next_input_id) {
            // Process the frame
            // Calculate metrics
            let (_left_raw, _right_raw, _field_raw, _pre_point_score, _com_x_opt, _com_y_opt) =
                calculate_frame_metrics(&mut current_frame, &config);

            // Calculate deltas
            calculate_deltas(&mut current_frame, history_buffer.last());
            // Record history
            history_buffer.push(FrameHistory {
                left_count: current_frame.left_count,
                right_count: current_frame.right_count,
                com_x: current_frame.com_x,
                com_y: current_frame.com_y,
                std_dev: current_frame.std_dev,
            });
            let cliff_results = cliff_state.push(current_frame.id, current_frame.pre_point_score);
            lookahead_buffer.push(current_frame);
            for (cid, is_cliff) in cliff_results {
                if is_cliff {
                    if let Some(f) = lookahead_buffer.iter_mut().find(|f| f.id == cid) {
                        f.is_cliff = true;
                    }
                }
            }
        }
        next_input_id += 1;
        if next_input_id > state.total_frames + 1000 {
            break;
        }
    }

    // 2. Flush remaining frames from lookahead_buffer
    while !lookahead_buffer.is_empty() {
        let frame = lookahead_buffer.remove(0);

        // Write final frames to CSV
        writeln!(
            features_csv,
            "{},{:.3},{:.3},{:.3},{:.3},{},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1}",
            frame.id,
            frame.left_count,
            frame.right_count,
            frame.field_count,
            frame.pre_point_score,
            if frame.is_cliff { 1 } else { 0 },
            frame.com_x.unwrap_or(-1.0),
            frame.com_y.unwrap_or(-1.0),
            frame.std_dev.unwrap_or(-1.0),
            frame.com_delta_x.unwrap_or(0.0),
            frame.com_delta_y.unwrap_or(0.0),
            frame.std_dev_delta.unwrap_or(0.0),
        )?;

        if frame.is_cliff {
            writeln!(
                points_csv,
                "{},{},{},{}",
                frame.id,
                if frame.is_cliff { 1 } else { 0 },
                if frame.left_emptied_first { 1 } else { 0 },
                if frame.right_emptied_first { 1 } else { 0 }
            )?;
        }

        let _ = tx_f.send(frame);
    }

    tracing::info!("Feature worker finished gracefully");
    Ok(())
}
