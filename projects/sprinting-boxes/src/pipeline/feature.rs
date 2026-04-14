use crate::geometry::is_point_in_polygon_robust;
use crate::pipeline::types::{
    compact_to_polygon, CompactCropData, CompactDetectionFile, CompactFrameData, CompactRegion,
    DetectedFrame, EnrichedDetection, ProcessingState,
};
use crate::scoring::{
    calculate_deltas, calculate_frame_metrics, calculate_pre_point_score, CliffDetectorConfig,
    CliffDetectorState, FrameHistory,
};
use anyhow::Result;
use crossbeam::channel::{Receiver, Sender};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

// Re-export FeatureConfig for backward compatibility with callers using crate::pipeline::feature::FeatureConfig
pub use crate::scoring::FeatureConfig;


/// Convert compact crop data back to CropResult for merging
fn compact_crop_to_result(
    compact: &CompactCropData,
    suffix: &str,
    regions: Option<Vec<CompactRegion>>,
    source_bbox: Option<crate::pipeline::types::BBox>,
) -> crate::pipeline::types::CropResult {
    let detections = compact
        .detections
        .iter()
        .map(|d| EnrichedDetection {
            bbox: crate::pipeline::types::BBox {
                x: d.x,
                y: d.y,
                w: d.w,
                h: d.h,
            },
            confidence: d.confidence,
            class_id: 0,
            class_name: Some("person".to_string()),
            in_end_zone: d.in_end_zone,
            in_field: d.in_field, // Preserve compact format values
        })
        .collect();

    let regions = if let Some(compact_regions) = regions {
        compact_regions
            .iter()
            .map(|r| crate::pipeline::types::RegionalPolygon {
                name: r.name.clone(),
                polygon: compact_to_polygon(&r.polygon),
                effective_polygon: compact_to_polygon(&r.polygon),
            })
            .collect()
    } else {
        Vec::new()
    };

    crate::pipeline::types::CropResult {
        suffix: suffix.to_string(),
        detections,
        original_polygon: Vec::new(),
        effective_polygon: Vec::new(),
        bbox: source_bbox.unwrap_or(crate::pipeline::types::BBox {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        }),
        image: None,
        regions,
    }
}

/// Convert compact frame data back to DetectedFrame (without metrics)
fn compact_frame_to_detected(compact: &CompactFrameData) -> DetectedFrame {
    let mut results = Vec::new();

    for (suffix, compact_crop) in &compact.crops {
        let regions = if suffix == "overview" {
            compact_crop.regions.clone()
        } else {
            None
        };

        let source_bbox = if suffix == "left" || suffix == "right" {
            compact_crop.source_bbox
        } else {
            None
        };

        results.push(compact_crop_to_result(
            compact_crop,
            suffix,
            regions,
            source_bbox,
        ));
    }

    // Correct in_field and in_end_zone flags from compact format
    // The compact format may have incorrect in_field/in_end_zone values from pull mode.
    // We need to refine these based on actual region polygons.
    for result in &mut results {
        if result.suffix == "overview" {
            // For overview crop, refine all detections based on region polygons
            let mut refined_detections = Vec::new();

            for mut detection in result.detections.drain(..) {
                let mut in_any_region = false;

                // Check each region
                for region in &result.regions {
                    if is_point_in_polygon_robust(
                        detection.bbox.x + detection.bbox.w / 2.0,
                        detection.bbox.y + detection.bbox.h / 2.0,
                        &region.effective_polygon,
                    ) {
                        in_any_region = true;
                        if region.name == "left" {
                            detection.in_end_zone = true;
                        } else if region.name == "right" {
                            detection.in_end_zone = true;
                        } else if region.name == "field" {
                            detection.in_field = true;
                        }
                        // Only set one flag per detection
                        break;
                    }
                }

                // If detection is not in any region, mark as neither
                if !in_any_region {
                    detection.in_field = false;
                    detection.in_end_zone = false;
                }

                refined_detections.push(detection);
            }

            result.detections = refined_detections;
        }
    }

    DetectedFrame {
        id: compact.id,
        results,
        // Metrics will be computed by the feature worker
        left_count: 0.0,
        right_count: 0.0,
        field_count: 0.0,
        pre_point_score: 0.0,
        is_cliff: false,
        left_emptied_first: false,
        right_emptied_first: false,
        maybe_false_positive: false,
        com_x: None,
        com_y: None,
        std_dev: None,
        com_delta_x: None,
        com_delta_y: None,
        std_dev_delta: None,
        detection_summary: None,
    }
}

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
    mode: crate::pipeline::types::PipelineMode,
) -> Result<()> {
    use std::io::Write;

    // Create CSV files based on mode
    let target_features = match mode {
        crate::pipeline::types::PipelineMode::Pull => "pull_features.csv",
        crate::pipeline::types::PipelineMode::Field => "features.csv",
    };
    let target_points = match mode {
        crate::pipeline::types::PipelineMode::Pull => "pull_points.csv",
        crate::pipeline::types::PipelineMode::Field => "points.csv",
    };

    let features_path = config.output_dir.join(target_features);
    let points_path = config.output_dir.join(target_points);
    tracing::info!(
        "Feature worker started. output_dir: {:?}, mode: {:?}",
        config.output_dir,
        mode
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

    let mut cliff_state = CliffDetectorState::new(CliffDetectorConfig::default());
    // Load pull detections for merging in Field mode
    let mut pull_data_map: BTreeMap<usize, DetectedFrame> = BTreeMap::new();
    if mode == crate::pipeline::types::PipelineMode::Field {
        let pull_path = config.output_dir.join("pull_detections.json");
        if pull_path.exists() {
            if let Ok(json) = std::fs::read_to_string(&pull_path) {
                // Try to load as compact format first
                if let Ok(compact_file) = serde_json::from_str::<CompactDetectionFile>(&json) {
                    for compact_frame in compact_file.frames {
                        let detected_frame = compact_frame_to_detected(&compact_frame);
                        pull_data_map.insert(detected_frame.id, detected_frame);
                    }
                    tracing::info!(
                        "Feature worker: Loaded {} frames from pull_detections.json (compact format)",
                        pull_data_map.len()
                    );
                } else if let Ok(pull_list) = serde_json::from_str::<Vec<DetectedFrame>>(&json) {
                    // Fallback to old format
                    for f in pull_list {
                        pull_data_map.insert(f.id, f);
                    }
                    tracing::info!(
                        "Feature worker: Loaded {} frames from pull_detections.json (legacy format)",
                        pull_data_map.len()
                    );
                }
            }
        }
    }

    for frame in rx {
        let start_inst = Instant::now();

        if !state.is_active.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        let mut frame = frame;
        if mode == crate::pipeline::types::PipelineMode::Field {
            if let Some(pull_frame) = pull_data_map.get(&frame.id) {
                // Merge pull detections (specifically end-zone ones) into the new frame
                if let Some(new_ov) = frame.results.iter_mut().find(|r| r.suffix == "overview") {
                    // Find the overview crop in the pull data and take all its detections that were in_end_zone.
                    if let Some(pull_ov) =
                        pull_frame.results.iter().find(|r| r.suffix == "overview")
                    {
                        let ez_dets: Vec<_> = pull_ov
                            .detections
                            .iter()
                            .filter(|d| d.in_end_zone)
                            .cloned()
                            .collect();
                        new_ov.detections.extend(ez_dets);
                    }
                }
            }
        }

        input_buffer.insert(frame.id, frame);

        while let Some(mut current_frame) = input_buffer.remove(&next_input_id) {
            let (_left_raw, _right_raw, _field_raw, _pre_point_score, _com_x_opt, _com_y_opt) =
                calculate_frame_metrics(&mut current_frame, &config, mode);

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
                    let end_idx = frame.id + config.lookahead_frames;

                    let mut left_zero_count = 0;
                    let mut right_zero_count = 0;
                    let mut left_emptied_at = None;
                    let mut right_emptied_at = None;

                    for i in start_idx..=end_idx {
                        if i >= history_buffer.len() {
                            break;
                        }
                        let h = &history_buffer[i];

                        if h.left_count == 0.0 {
                            left_zero_count += 1;
                            if left_zero_count >= 2 && left_emptied_at.is_none() {
                                left_emptied_at = Some(i);
                            }
                        } else {
                            left_zero_count = 0;
                        }

                        if h.right_count == 0.0 {
                            right_zero_count += 1;
                            if right_zero_count >= 2 && right_emptied_at.is_none() {
                                right_emptied_at = Some(i);
                            }
                        } else {
                            right_zero_count = 0;
                        }
                    }

                    match (left_emptied_at, right_emptied_at) {
                        (Some(l), Some(r)) => {
                            if l < r {
                                frame.left_emptied_first = true;
                            } else if r < l {
                                frame.right_emptied_first = true;
                            } else {
                                // Both emptied at the same time: look back for tie-breaker
                                let mut winner_found = false;
                                for back_idx in (0..l).rev() {
                                    if back_idx >= history_buffer.len() {
                                        break;
                                    }
                                    let h = &history_buffer[back_idx];
                                    if h.left_count < h.right_count {
                                        frame.left_emptied_first = true;
                                        winner_found = true;
                                        break;
                                    } else if h.right_count < h.left_count {
                                        frame.right_emptied_first = true;
                                        winner_found = true;
                                        break;
                                    }
                                }
                                if !winner_found {
                                    // True tie if no difference found
                                    frame.left_emptied_first = true;
                                    frame.right_emptied_first = true;
                                }
                            }
                        }
                        (Some(_), None) => frame.left_emptied_first = true,
                        (None, Some(_)) => frame.right_emptied_first = true,
                        (None, None) => frame.maybe_false_positive = true,
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
                calculate_frame_metrics(&mut current_frame, &config, mode);

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

