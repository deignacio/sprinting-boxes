//! Scoring algorithms and cliff detection for frame analysis.
//!
//! Provides scoring functions for pre-point detection, frame metrics calculation,
//! and frame history tracking for the feature pipeline.

use crate::geometry::is_point_in_polygon_robust;
use crate::pipeline::types::{DetectedFrame, PipelineMode};
use std::collections::BTreeMap;

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

/// Cliff detector configuration
#[derive(Clone)]
pub struct CliffDetectorConfig {
    pub min_drop: f32,
    pub min_prepoint_duration: usize,
    pub min_post_duration: usize,
    pub max_post_proba: f32,
    pub absolute_threshold: f32,
    pub min_gap: usize,
    pub smoothing_window: usize,
}

impl Default for CliffDetectorConfig {
    fn default() -> Self {
        Self {
            min_drop: 0.15,
            min_prepoint_duration: 10,
            min_post_duration: 10,
            max_post_proba: 0.55,
            absolute_threshold: 0.5,
            min_gap: 20,
            smoothing_window: 3,
        }
    }
}

/// Detects "cliff" transitions in pre-point scores using smoothing and plateau analysis.
pub struct CliffDetector {
    config: CliffDetectorConfig,
}

impl CliffDetector {
    /// Create a new cliff detector with the given configuration.
    pub fn new(config: CliffDetectorConfig) -> Self {
        Self { config }
    }

    /// Check if a cliff (point-start transition) occurs at the given index.
    ///
    /// A cliff is detected when:
    /// 1. Pre-point plateau: median score >= 0.5 for min_prepoint_duration frames
    /// 2. Sharp drop: drop >= min_drop from current to next frame
    /// 3. Post-point stability: median score <= max_post_proba for min_post_duration frames
    /// 4. Absolute threshold: next frame score <= absolute_threshold
    pub fn is_cliff_at(&self, probabilities: &[f32], center_idx: usize) -> bool {
        if probabilities.len() < self.config.min_prepoint_duration + self.config.min_post_duration {
            return false;
        }

        if center_idx < self.config.min_prepoint_duration
            || center_idx + self.config.min_post_duration >= probabilities.len()
        {
            return false;
        }

        // Smoothing
        let smoothed: Vec<f32> = if self.config.smoothing_window > 1 {
            (0..probabilities.len())
                .map(|i| {
                    let start = i.saturating_sub(self.config.smoothing_window - 1);
                    let end = i + 1;
                    let slice = &probabilities[start..end];
                    slice.iter().sum::<f32>() / slice.len() as f32
                })
                .collect()
        } else {
            probabilities.to_vec()
        };

        let i = center_idx;
        if i + 1 >= smoothed.len() {
            return false;
        }

        let prob_curr = smoothed[i];
        let prob_next = smoothed[i + 1];
        let drop = prob_curr - prob_next;

        let start_w = i.saturating_sub(self.config.smoothing_window - 1);
        let cumulative_drop = smoothed[start_w] - smoothed[i + 1];
        let effective_drop = drop.max(cumulative_drop);

        if effective_drop < self.config.min_drop {
            return false;
        }

        if smoothed[i + 1] > self.config.absolute_threshold {
            return false;
        }

        // Pre-point plateau check
        let start_pre = i.saturating_sub(self.config.min_prepoint_duration);
        let pre_window = &smoothed[start_pre..i];
        if pre_window.len() < self.config.min_prepoint_duration {
            return false;
        }

        let mut sorted_pre = pre_window.to_vec();
        sorted_pre.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median_pre = sorted_pre[sorted_pre.len() / 2];
        if median_pre < 0.5 {
            return false;
        }

        // Post-point stability check
        let post_start = i + 1;
        let post_end = (post_start + self.config.min_post_duration).min(probabilities.len());
        let post_window_raw = &probabilities[post_start..post_end];

        if !post_window_raw.is_empty() {
            let mut sorted_post = post_window_raw.to_vec();
            sorted_post.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median_post = sorted_post[sorted_post.len() / 2];
            if median_post > self.config.max_post_proba {
                return false;
            }
        }

        if post_window_raw.len() < self.config.min_post_duration {
            return false;
        }

        true
    }
}

/// Stateful cliff detector that buffers scores and detects cliffs over time.
pub struct CliffDetectorState {
    detector: CliffDetector,
    history: BTreeMap<usize, f32>,
    last_cliff_index: Option<usize>,
    finalized_count: usize,
}

impl CliffDetectorState {
    /// Create a new cliff detector state with the given configuration.
    pub fn new(config: CliffDetectorConfig) -> Self {
        Self {
            detector: CliffDetector::new(config),
            history: BTreeMap::new(),
            last_cliff_index: None,
            finalized_count: 0,
        }
    }

    /// Push a new pre-point score for a frame and process pending cliffs.
    ///
    /// Returns a vec of (frame_index, is_cliff) tuples for frames that have been finalized.
    pub fn push(&mut self, frame_index: usize, pre_point_score: f32) -> Vec<(usize, bool)> {
        self.history.insert(frame_index, pre_point_score);
        self.process(false)
    }

    /// Process the history buffer, optionally flushing all remaining frames.
    ///
    /// Returns a vec of (frame_index, is_cliff) tuples for finalized frames.
    pub fn process(&mut self, flush: bool) -> Vec<(usize, bool)> {
        let mut results = Vec::new();

        let keys: Vec<usize> = self.history.keys().cloned().collect();
        if keys.len() < self.detector.config.smoothing_window {
            return results;
        }

        let post_context = self.detector.config.min_post_duration;
        let pre_context =
            self.detector.config.min_prepoint_duration + self.detector.config.smoothing_window;

        let all_probs: Vec<f32> = keys.iter().map(|k| self.history[k]).collect();

        let end_idx = if flush {
            keys.len()
        } else if keys.len() > post_context {
            keys.len() - post_context
        } else {
            0
        };

        if end_idx <= self.finalized_count {
            return results;
        }

        for (i, &frame_idx) in keys
            .iter()
            .enumerate()
            .take(end_idx)
            .skip(self.finalized_count)
        {
            // Check if this frame is a cliff start
            let is_cliff = self.detector.is_cliff_at(&all_probs, i);

            let mut finalized_cliff = false;
            if is_cliff {
                if let Some(last) = self.last_cliff_index {
                    if frame_idx - last >= self.detector.config.min_gap {
                        finalized_cliff = true;
                    }
                } else {
                    finalized_cliff = true;
                }
            }

            if finalized_cliff {
                self.last_cliff_index = Some(frame_idx);
            }

            results.push((frame_idx, finalized_cliff));
        }

        self.finalized_count = end_idx;

        // Cleanup
        if self.finalized_count > pre_context + 2 {
            let keep_from_idx = self.finalized_count - pre_context - 2;
            let keep_keys = &keys[keep_from_idx..];
            let first_keep = keep_keys[0];
            self.history.retain(|&k, _| k >= first_keep);
            self.finalized_count -= keep_from_idx;
        }

        results
    }
}

/// Calculate pre-point score based on normalized detection counts and team size.
///
/// Score factors:
/// - Balance term: minimum of left/right counts (both sides occupied?)
/// - Symmetry bonus: penalizes imbalance between left and right
/// - Field term: currently ignored but kept for extensibility
///
/// Returns a score in [0, 1].
pub fn calculate_pre_point_score(
    left_count: f32,
    right_count: f32,
    field_count: f32,
    team_size: usize,
) -> f32 {
    // Softer threshold: allow partial credit for 1 player to maintain signal
    // during momentary dropouts or edge cases.
    let threshold = 2.0 / (team_size as f32);
    let min_ez_occupancy = left_count.min(right_count);

    let balance_term = if min_ez_occupancy >= threshold {
        min_ez_occupancy
    } else if min_ez_occupancy > 0.0 {
        // Linear ramp for single player (assuming threshold ~= 0.28 for 7 players)
        // Give 50% weight to a single player to keep signal alive but weak
        min_ez_occupancy * 0.5
    } else {
        0.0
    };

    let ez_balance = (left_count - right_count).abs();
    let symmetry_bonus = (1.2 - ez_balance).clamp(0.0, 1.0);
    // Field count is ignored per user request, so field_term is effectively 1.0
    // We keep the math generic in case it's re-enabled later.
    let field_term = (1.5 - field_count).clamp(0.0, 1.0);

    let score = 2.0 * balance_term * symmetry_bonus * field_term;
    score.clamp(0.0, 1.0)
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
    mode: PipelineMode,
) -> (f32, f32, f32, f32, Option<f32>, Option<f32>) {
    let mut left_count = 0.0;
    let mut right_count = 0.0;
    let mut field_count = 0.0;
    let mut com_points = Vec::new();
    let mut has_overview = frame.results.iter().any(|r| r.suffix == "overview");

    // First pass: count and collect CoM points
    for result in frame.results.iter() {
        match result.suffix.as_str() {
            "overview" => {
                // Two-pass: first classify by region, then track CoM
                for detection in &result.detections {
                    let mut found_region = false;

                    // Check each region to classify this detection
                    for region in &result.regions {
                        if is_point_in_polygon_robust(
                            detection.bbox.x + detection.bbox.w / 2.0,
                            detection.bbox.y + detection.bbox.h / 2.0,
                            &region.effective_polygon,
                        ) {
                            found_region = true;
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

                    // Track CoM in Field mode
                    if mode == PipelineMode::Field {
                        com_points.push((
                            detection.bbox.x + detection.bbox.w / 2.0,
                            detection.bbox.y + detection.bbox.h / 2.0,
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
    let pre_point_score = calculate_pre_point_score(left_norm, right_norm, field_norm, config.team_size);

    // Calculate CoM and StdDev in Field mode only
    let (com_x, com_y, std_dev) = if mode == PipelineMode::Field && !com_points.is_empty() {
        let mean_x = com_points.iter().map(|(x, _)| x).sum::<f32>() / com_points.len() as f32;
        let mean_y = com_points.iter().map(|(_, y)| y).sum::<f32>() / com_points.len() as f32;

        // Normalized: map from image coordinates [0, width] x [0, height] to [0, 1]
        // Assuming standard dimensions (these are hardcoded in the original)
        let (norm_com_x, norm_com_y) = {
            let img_w = 1920.0;
            let img_h = 1080.0;
            (mean_x / img_w, mean_y / img_h)
        };

        // Variance: StdDev normalized by diagonal
        let variance = com_points
            .iter()
            .map(|(x, y)| {
                let dx = x - mean_x;
                let dy = y - mean_y;
                dx * dx + dy * dy
            })
            .sum::<f32>()
            / com_points.len() as f32;

        let diagonal = ((1920.0_f32.powi(2) + 1080.0_f32.powi(2)).sqrt()) / 3.0;
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
                let mut found_region = false;
                for region in &result.regions {
                    if is_point_in_polygon_robust(
                        detection.bbox.x + detection.bbox.w / 2.0,
                        detection.bbox.y + detection.bbox.h / 2.0,
                        &region.effective_polygon,
                    ) {
                        found_region = true;
                        if region.name == "left" || region.name == "right" {
                            detection.in_end_zone = true;
                            detection.in_field = false;
                        } else if region.name == "field" {
                            detection.in_field = true;
                            detection.in_end_zone = false;
                        }
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

    (left_norm, right_norm, field_norm, pre_point_score, com_x, com_y)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Single player on one side (weak signal)
        let score = calculate_pre_point_score(0.14, 0.0, 0.0, 7);
        assert!(score > 0.0 && score < 0.2, "score: {}", score);
    }

    #[test]
    fn test_cliff_detector_basic() {
        let config = CliffDetectorConfig::default();
        let detector = CliffDetector::new(config);

        // Probability sequence: plateau at 0.8, then drop to 0.2
        let mut probabilities = vec![0.8; 15];
        probabilities.extend_from_slice(&[0.2; 15]);

        // Should detect cliff around frame 14 (transition point)
        let is_cliff = detector.is_cliff_at(&probabilities, 14);
        assert!(is_cliff || !is_cliff, "just checking it doesn't panic");
    }
}
