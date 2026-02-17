use crate::pipeline::geometry::is_point_in_polygon_robust;
use crate::pipeline::types::{DetectedFrame, ProcessingState};
use anyhow::Result;
use crossbeam::channel::{Receiver, Sender};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

/// Configuration for feature extraction and cliff detection
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

/// Cliff detector configuration
#[derive(Clone)]
struct CliffDetectorConfig {
    min_drop: f32,
    min_prepoint_duration: usize,
    min_post_duration: usize,
    max_post_proba: f32,
    absolute_threshold: f32,
    min_gap: usize,
    smoothing_window: usize,
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

struct CliffDetector {
    config: CliffDetectorConfig,
}

impl CliffDetector {
    fn new(config: CliffDetectorConfig) -> Self {
        Self { config }
    }

    fn is_cliff_at(&self, probabilities: &[f32], center_idx: usize) -> bool {
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

struct CliffDetectorState {
    detector: CliffDetector,
    history: BTreeMap<usize, f32>,
    last_cliff_index: Option<usize>,
    finalized_count: usize,
}

impl CliffDetectorState {
    fn new(config: CliffDetectorConfig) -> Self {
        Self {
            detector: CliffDetector::new(config),
            history: BTreeMap::new(),
            last_cliff_index: None,
            finalized_count: 0,
        }
    }

    fn push(&mut self, frame_index: usize, pre_point_score: f32) -> Vec<(usize, bool)> {
        self.history.insert(frame_index, pre_point_score);
        self.process(false)
    }

    fn process(&mut self, flush: bool) -> Vec<(usize, bool)> {
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

struct FrameHistory {
    left_count: f32,
    right_count: f32,
}

/// Calculate pre-point score based on normalized detection counts
fn calculate_pre_point_score(
    left_count: f32,
    right_count: f32,
    field_count: f32,
    team_size: usize,
) -> f32 {
    let min_ez_occupancy = left_count.min(right_count);
    let threshold = 2.0 / (team_size as f32);

    let balance_term = if min_ez_occupancy >= threshold {
        min_ez_occupancy
    } else {
        0.0
    };

    let ez_balance = (left_count - right_count).abs();
    let symmetry_bonus = (1.2 - ez_balance).clamp(0.0, 1.0);
    let field_term = (1.5 - field_count).clamp(0.0, 1.0);

    let score = 2.0 * balance_term * symmetry_bonus * field_term;
    score.clamp(0.0, 1.0)
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
    tx: Sender<DetectedFrame>,
    config: FeatureConfig,
    state: Arc<ProcessingState>,
) -> Result<()> {
    use std::io::Write;

    // Create CSV files
    let features_path = config.output_dir.join("features.csv");
    tracing::info!(
        "Feature worker started. output_dir: {:?}",
        config.output_dir
    );
    let points_path = config.output_dir.join("points.csv");

    let mut features_csv = std::fs::File::create(&features_path)?;
    writeln!(
        features_csv,
        "frame_index,left_count,right_count,field_count,pre_point_score,is_cliff"
    )?;

    let mut points_csv = std::fs::File::create(&points_path)?;
    writeln!(
        points_csv,
        "frame_index,is_cliff,left_side_emptied_first,right_side_emptied_first"
    )?;

    let mut cliff_state = CliffDetectorState::new(CliffDetectorConfig::default());
    let mut input_buffer: BTreeMap<usize, DetectedFrame> = BTreeMap::new();
    let mut next_input_id = 0;
    let mut lookahead_buffer: Vec<DetectedFrame> = Vec::new();
    let mut history_buffer: Vec<FrameHistory> = Vec::new();

    for frame in rx {
        let start_inst = Instant::now();

        if !state.is_active.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        input_buffer.insert(frame.id, frame);

        while let Some(mut current_frame) = input_buffer.remove(&next_input_id) {
            // Calculate normalized counts
            let mut left_count_raw = 0.0;
            let mut right_count_raw = 0.0;
            let mut field_count_raw = 0.0;

            for res in &mut current_frame.results {
                let mut count = 0;
                for d in &mut res.detections {
                    let x = d.bbox.x;
                    let y = d.bbox.y;
                    let w = d.bbox.w;
                    let h = d.bbox.h;
                    let bottom_center_x = x + w / 2.0;
                    let bottom_center_y = y + h;

                    d.is_counted = is_point_in_polygon_robust(
                        bottom_center_x,
                        bottom_center_y,
                        &res.effective_polygon,
                    );

                    if d.is_counted {
                        count += 1;
                    }
                }

                let norm = count as f32 / config.team_size as f32;
                match res.suffix.as_str() {
                    "left" => left_count_raw = norm,
                    "right" => right_count_raw = norm,
                    "field" => field_count_raw = norm,
                    _ => {}
                }
            }

            current_frame.left_count = left_count_raw;
            current_frame.right_count = right_count_raw;
            current_frame.field_count = field_count_raw;
            current_frame.pre_point_score = calculate_pre_point_score(
                left_count_raw,
                right_count_raw,
                field_count_raw,
                config.team_size,
            );

            // Record history
            history_buffer.push(FrameHistory {
                left_count: left_count_raw,
                right_count: right_count_raw,
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
                    "{},{:.3},{:.3},{:.3},{:.3},{}",
                    frame.id,
                    frame.left_count,
                    frame.right_count,
                    frame.field_count,
                    frame.pre_point_score,
                    if frame.is_cliff { 1 } else { 0 }
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

                if tx.send(frame).is_err() {
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
            let mut left_count_raw = 0.0;
            let mut right_count_raw = 0.0;
            let mut field_count_raw = 0.0;
            for res in &mut current_frame.results {
                let mut count = 0;
                for d in &mut res.detections {
                    let bc_x = d.bbox.x + d.bbox.w / 2.0;
                    let bc_y = d.bbox.y + d.bbox.h;
                    d.is_counted = is_point_in_polygon_robust(bc_x, bc_y, &res.effective_polygon);
                    if d.is_counted {
                        count += 1;
                    }
                }
                let norm = count as f32 / config.team_size as f32;
                match res.suffix.as_str() {
                    "left" => left_count_raw = norm,
                    "right" => right_count_raw = norm,
                    "field" => field_count_raw = norm,
                    _ => {}
                }
            }
            current_frame.left_count = left_count_raw;
            current_frame.right_count = right_count_raw;
            current_frame.field_count = field_count_raw;
            current_frame.pre_point_score = calculate_pre_point_score(
                left_count_raw,
                right_count_raw,
                field_count_raw,
                config.team_size,
            );

            history_buffer.push(FrameHistory {
                left_count: left_count_raw,
                right_count: right_count_raw,
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
            "{},{:.3},{:.3},{:.3},{:.3},{}",
            frame.id,
            frame.left_count,
            frame.right_count,
            frame.field_count,
            frame.pre_point_score,
            if frame.is_cliff { 1 } else { 0 }
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

        let _ = tx.send(frame);
    }

    tracing::info!("Feature worker finished gracefully");
    Ok(())
}
