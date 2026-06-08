//! Scoring algorithms and cliff detection for frame analysis.
//!
//! Provides scoring functions for pre-point detection, frame metrics calculation,
//! and frame history tracking for the feature pipeline.

use crate::geometry::is_point_in_polygon_robust;
use crate::pipeline::types::DetectedFrame;
use ultimate_event_detection::{pre_point_score, EndZoneOccupancy};

/// Feature extraction configuration (re-exported from feature module for convenience)
pub struct FeatureConfig {
    pub team_size: usize,
    pub lookback_frames: usize,
    pub lookahead_frames: usize,
    pub output_dir: std::path::PathBuf,
}

impl Default for FeatureConfig {
    fn default() -> Self {
        Self {
            team_size: 7,
            lookback_frames: 10,
            lookahead_frames: 15,
            output_dir: std::path::PathBuf::from("."),
        }
    }
}

/// Frame history snapshot for computing deltas
#[derive(Clone)]
pub struct FrameHistory {
    pub left_count: f32,
    pub right_count: f32,
    pub com_x: Option<f32>,
    pub com_y: Option<f32>,
    pub std_dev: Option<f32>,
}

/// Calculate pre-point score based on normalized detection counts and team size.
///
/// Returns a score in [0, 1].
pub fn calculate_pre_point_score(
    left_count: f32,
    right_count: f32,
    field_count: f32,
    team_size: usize,
) -> f32 {
    pre_point_score(
        &EndZoneOccupancy { left: left_count, right: right_count, field: field_count },
        team_size as u32,
    )
}

/// Calculate deltas in center-of-mass and std dev from previous frame.
pub fn calculate_deltas(frame: &mut DetectedFrame, prev_history: Option<&FrameHistory>) {
    if let Some(prev) = prev_history {
        if let (Some(curr_x), Some(prev_x)) = (frame.com_x, prev.com_x) {
            frame.com_delta_x = Some(curr_x - prev_x);
        }
        if let (Some(curr_y), Some(prev_y)) = (frame.com_y, prev.com_y) {
            frame.com_delta_y = Some(curr_y - prev_y);
        }
        if let (Some(curr_std), Some(prev_std)) = (frame.std_dev, prev.std_dev) {
            frame.std_dev_delta = Some(curr_std - prev_std);
        }
    }
}

/// Calculate normalized player counts and center-of-mass for a frame.
///
/// Returns (left_count, right_count, field_count, pre_point_score, com_x, com_y)
pub fn calculate_frame_metrics(
    frame: &mut DetectedFrame,
    config: &FeatureConfig,
) -> (f32, f32, f32, f32, Option<f32>, Option<f32>) {
    let mut left_count = 0.0;
    let mut right_count = 0.0;
    let mut field_count = 0.0;
    let mut com_points = Vec::new();
    let has_overview = frame.results.iter().any(|r| r.suffix == "overview");

    // First pass: count and collect CoM points
    for result in frame.results.iter() {
        match result.suffix.as_str() {
            "overview" => {
                // Two-pass: first classify by region, then track CoM
                for detection in &result.detections {
                    let mut in_valid_region = false;

                    // Check each region to classify this detection
                    for region in &result.regions {
                        if is_point_in_polygon_robust(
                            detection.bbox.x + detection.bbox.w / 2.0,
                            detection.bbox.y + detection.bbox.h,
                            &region.effective_polygon,
                        ) {
                            in_valid_region = true;
                            if region.name == "left" {
                                left_count += 1.0;
                            } else if region.name == "right" {
                                right_count += 1.0;
                            } else if region.name == "field" {
                                field_count += 1.0;
                            }
                            break; // Only classify once
                        }
                    }

                    // Only include detections in valid regions for CoM calculation
                    if in_valid_region {
                        com_points.push((
                            detection.bbox.x + detection.bbox.w / 2.0,
                            detection.bbox.y + detection.bbox.h,
                        ));
                    }
                }
            }
            "left" | "right" => {
                // Skip EZ crops if overview was already processed (avoid double-counting)
                if has_overview {
                    continue;
                }
                // Fallback: count directly if no overview crop
                for detection in &result.detections {
                    if detection.in_end_zone {
                        if result.suffix == "left" {
                            left_count += 1.0;
                        } else {
                            right_count += 1.0;
                        }
                    }
                }
            }
            _ => {
                // Legacy: use effective_polygon if regions unavailable
                for detection in &result.detections {
                    if is_point_in_polygon_robust(
                        detection.bbox.x + detection.bbox.w / 2.0,
                        detection.bbox.y + detection.bbox.h / 2.0,
                        &result.effective_polygon,
                    ) {
                        if result.suffix == "left" || result.suffix == "right" {
                            if result.suffix == "left" {
                                left_count += 1.0;
                            } else {
                                right_count += 1.0;
                            }
                        } else {
                            field_count += 1.0;
                        }
                    }
                }
            }
        }
    }

    // Normalize counts by team size
    let left_norm = left_count / config.team_size as f32;
    let right_norm = right_count / config.team_size as f32;
    let field_norm = field_count / config.team_size as f32;

    // Calculate pre-point score
    let pre_point_score =
        calculate_pre_point_score(left_norm, right_norm, field_norm, config.team_size);

    let (com_x, com_y, std_dev) = if !com_points.is_empty() {
        let mean_x = com_points.iter().map(|(x, _)| x).sum::<f32>() / com_points.len() as f32;
        let mean_y = com_points.iter().map(|(_, y)| y).sum::<f32>() / com_points.len() as f32;

        // Normalized: map from image coordinates [0, width] x [0, height] to [0, 1]
        // Use actual crop dimensions from the overview CropResult bbox.
        let (img_w, img_h) = frame.overview_crop_dimensions();
        let (norm_com_x, norm_com_y) = (mean_x / img_w, mean_y / img_h);

        // Variance: StdDev normalized by diagonal of the actual crop.
        let variance = com_points
            .iter()
            .map(|(x, y)| {
                let dx = x - mean_x;
                let dy = y - mean_y;
                dx * dx + dy * dy
            })
            .sum::<f32>()
            / com_points.len() as f32;

        let (img_w, img_h) = frame.overview_crop_dimensions();
        let diagonal = ((img_w.powi(2) + img_h.powi(2)).sqrt()) / 3.0;
        let std_dev = variance.sqrt() / diagonal;

        (Some(norm_com_x), Some(norm_com_y), Some(std_dev))
    } else {
        (None, None, None)
    };

    // Second pass: update frame mutably with computed metrics
    frame.left_count = left_norm;
    frame.right_count = right_norm;
    frame.field_count = field_norm;
    frame.pre_point_score = pre_point_score;
    frame.com_x = com_x;
    frame.com_y = com_y;
    frame.std_dev = std_dev;

    // Now update detection flags in a separate mutable pass
    for result in &mut frame.results {
        if result.suffix == "overview" {
            for detection in &mut result.detections {
                let ground_x = detection.bbox.x + detection.bbox.w / 2.0;
                let ground_y = detection.bbox.y + detection.bbox.h;
                let mut found_region = false;
                for region in &result.regions {
                    if is_point_in_polygon_robust(ground_x, ground_y, &region.effective_polygon) {
                        found_region = true;
                        if region.name == "left" || region.name == "right" {
                            detection.in_end_zone = true;
                            detection.in_field = false;
                        } else if region.name == "field" {
                            detection.in_field = true;
                            detection.in_end_zone = false;
                        }
                        tracing::trace!(
                            ground_x,
                            ground_y,
                            bbox_x = detection.bbox.x,
                            bbox_y = detection.bbox.y,
                            bbox_w = detection.bbox.w,
                            bbox_h = detection.bbox.h,
                            region = region.name.as_str(),
                            in_end_zone = detection.in_end_zone,
                            "detection region assignment"
                        );
                        break;
                    }
                }
                if !found_region {
                    detection.in_field = false;
                    detection.in_end_zone = false;
                }
            }
        }
    }

    (
        left_norm,
        right_norm,
        field_norm,
        pre_point_score,
        com_x,
        com_y,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ultimate_event_detection::CliffDetectorConfig;

    #[test]
    fn test_calculate_pre_point_score_both_sides() {
        // Both sides occupied equally
        let score = calculate_pre_point_score(0.5, 0.5, 0.0, 7);
        assert!(score > 0.9, "score: {}", score);
    }

    #[test]
    fn test_calculate_pre_point_score_empty() {
        // Empty end-zones
        let score = calculate_pre_point_score(0.0, 0.0, 0.0, 7);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_calculate_pre_point_score_single_player() {
        // One player in each end zone: weak but non-zero signal
        let score = calculate_pre_point_score(0.14, 0.14, 0.0, 7);
        assert!(score > 0.0 && score < 0.2, "score: {}", score);
    }

    #[test]
    fn test_cliff_detector_basic() {
        let config = CliffDetectorConfig::default();

        // Probability sequence: plateau at 0.8, then drop to 0.2
        let mut probabilities = vec![0.8; 15];
        probabilities.extend_from_slice(&[0.2; 15]);

        // Just checking it doesn't panic
        let _ = ultimate_event_detection::is_cliff_at(&config, &probabilities, 14);
    }
}
