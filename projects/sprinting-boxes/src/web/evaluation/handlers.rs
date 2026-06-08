use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use rayon::prelude::*;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::Arc;
use std::time::Instant;

use ultimate_event_detection::{CliffDetector, CliffDetectorConfig, GpuCliffDetector};
use crate::run_context::list_runs;
use crate::web::server::AppState;

use super::models::{AggregatedMetrics, DetectorConfigParams, EvaluationMetrics, FNCause, FPCause, GlobalSweepRequest, GlobalSweepResponse};

/// Raw frame features from features.csv
struct FrameFeatures {
    frame_index: usize,
    left_count: f32,
    right_count: f32,
    field_count: f32,
    pre_point_score: f32,
}

/// Load frame features from features.csv
fn load_features(
    features_path: &std::path::Path,
) -> Result<Vec<(usize, f32)>, StatusCode> {
    let content = fs::read_to_string(features_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut reader = csv::Reader::from_reader(content.as_bytes());
    let mut frames = Vec::new();

    for result in reader.records() {
        let record = result.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let frame_index: usize = record
            .get(0)
            .ok_or(StatusCode::BAD_REQUEST)?
            .parse()
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let pre_point_score: f32 = record
            .get(4)
            .ok_or(StatusCode::BAD_REQUEST)?
            .parse()
            .map_err(|_| StatusCode::BAD_REQUEST)?;

        frames.push((frame_index, pre_point_score));
    }

    frames.sort_by_key(|f| f.0);
    Ok(frames)
}

/// Load raw frame features with left/right/field counts
fn load_raw_features(
    features_path: &std::path::Path,
) -> Result<Vec<FrameFeatures>, StatusCode> {
    let content = fs::read_to_string(features_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut reader = csv::Reader::from_reader(content.as_bytes());
    let mut frames = Vec::new();

    for result in reader.records() {
        let record = result.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let frame_index: usize = record
            .get(0)
            .ok_or(StatusCode::BAD_REQUEST)?
            .parse()
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let left_count: f32 = record
            .get(1)
            .ok_or(StatusCode::BAD_REQUEST)?
            .parse()
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let right_count: f32 = record
            .get(2)
            .ok_or(StatusCode::BAD_REQUEST)?
            .parse()
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let field_count: f32 = record
            .get(3)
            .ok_or(StatusCode::BAD_REQUEST)?
            .parse()
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let pre_point_score: f32 = record
            .get(4)
            .ok_or(StatusCode::BAD_REQUEST)?
            .parse()
            .map_err(|_| StatusCode::BAD_REQUEST)?;

        frames.push(FrameFeatures {
            frame_index,
            left_count,
            right_count,
            field_count,
            pre_point_score,
        });
    }

    frames.sort_by_key(|f| f.frame_index);
    Ok(frames)
}

/// Recalculate pre_point_score with custom field_onset
fn recalculate_score_with_field_onset(
    left_count: f32,
    right_count: f32,
    field_count: f32,
    field_onset: f32,
) -> f32 {
    const TEAM_SIZE: f32 = 7.0;
    let threshold = 2.0 / TEAM_SIZE;

    let min_ez = left_count.min(right_count);
    let balance = if min_ez >= threshold {
        min_ez
    } else if min_ez > 0.0 {
        min_ez * 0.5
    } else {
        0.0
    };

    let symmetry = ((1.2 - (left_count - right_count).abs()).max(0.0)).min(1.0);
    let field_term = ((field_onset - field_count).max(0.0)).min(1.0);

    (2.0 * balance * symmetry * field_term).min(1.0)
}

/// Classify why a false negative wasn't detected
fn classify_fn(
    frame_idx: usize,
    frames: &[(usize, f32)],
    detected_cliffs: &HashSet<usize>,
    config: &DetectorConfigParams,
) -> FNCause {
    // Check if suppressed by min_gap from a nearby detected cliff
    for &cliff_idx in detected_cliffs {
        if cliff_idx != frame_idx && (cliff_idx as i32 - frame_idx as i32).abs() < config.min_gap as i32 {
            return FNCause::MinGapSuppressed;
        }
    }

    // Find position of frame_idx in the frames list
    let idx_pos = match frames.iter().position(|(fi, _)| *fi == frame_idx) {
        Some(pos) => pos,
        None => return FNCause::Unknown,
    };

    let n = frames.len();
    let pre_start = if idx_pos >= config.min_prepoint_duration {
        idx_pos - config.min_prepoint_duration
    } else {
        0
    };
    let post_end = std::cmp::min(idx_pos + config.min_post_duration + 1, n);

    // Get pre-window scores (smoothed per Python implementation)
    let pre_scores: Vec<f32> = frames[pre_start..idx_pos]
        .iter()
        .map(|(_, score)| *score)
        .collect();

    if !pre_scores.is_empty() {
        let median_pre = {
            let mut sorted = pre_scores.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            sorted[sorted.len() / 2]
        };

        if median_pre < 0.5 {
            return FNCause::NoPrePointPlateau;
        }
    }

    // Get post-window scores (raw scores)
    let post_scores: Vec<f32> = frames[idx_pos + 1..post_end]
        .iter()
        .map(|(_, score)| *score)
        .collect();

    if !post_scores.is_empty() {
        let mut sorted = post_scores.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median_post = sorted[sorted.len() / 2];

        if median_post > config.max_post_proba {
            return FNCause::PostScoreTooHigh;
        }
    }

    // Check drop between current and next frame
    if idx_pos + 1 < n {
        let drop = frames[idx_pos].1 - frames[idx_pos + 1].1;
        if drop < config.min_drop {
            return FNCause::PlateauButNoDrop;
        }
    }

    FNCause::Unknown
}

/// Classify why a false positive was detected
fn classify_fp(
    frame_idx: usize,
    frames: &[(usize, f32)],
    ground_truth: &HashSet<usize>,
    config: &DetectorConfigParams,
) -> FPCause {
    // Find position of frame_idx in the frames list
    let idx_pos = match frames.iter().position(|(fi, _)| *fi == frame_idx) {
        Some(pos) => pos,
        None => return FPCause::Unknown,
    };

    let n = frames.len();
    let score_at_fp = frames[idx_pos].1;
    let pre_start = if idx_pos >= config.min_prepoint_duration {
        idx_pos - config.min_prepoint_duration
    } else {
        0
    };
    let post_end = std::cmp::min(idx_pos + config.min_post_duration + 1, n);

    // Get pre-window scores
    let pre_scores: Vec<f32> = frames[pre_start..idx_pos]
        .iter()
        .map(|(_, score)| *score)
        .collect();

    let median_pre = if !pre_scores.is_empty() {
        let mut sorted = pre_scores.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted[sorted.len() / 2]
    } else {
        0.0
    };

    // Get post-window scores
    let post_scores: Vec<f32> = frames[idx_pos + 1..post_end]
        .iter()
        .map(|(_, score)| *score)
        .collect();

    let median_post = if !post_scores.is_empty() {
        let mut sorted = post_scores.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted[sorted.len() / 2]
    } else {
        0.0
    };

    // Check if nearby frames have sustained high scores (incomplete transition)
    // If pre-window median is high but post is also relatively high, it's not a clean break
    if median_pre > 0.5 && median_post > 0.3 {
        return FPCause::IncompleteTransition;
    }

    // Check for low quality data (spiky/noisy scores)
    // If the FP score is an outlier compared to surrounding frames
    if idx_pos > 0 && idx_pos + 1 < n {
        let prev_score = frames[idx_pos - 1].1;
        let next_score = frames[idx_pos + 1].1;
        let avg_neighbor = (prev_score + next_score) / 2.0;

        // If the FP frame score is much higher than neighbors, it's likely noisy data
        if score_at_fp > avg_neighbor * 1.3 && avg_neighbor < 0.5 {
            return FPCause::LowQualityData;
        }
    }

    // Check if there are nearby ground truth points (might be part of a sequence)
    let mut has_nearby_truth = false;
    for i in (frame_idx as i32 - 10)..=(frame_idx as i32 + 10) {
        if i >= 0 && i != frame_idx as i32 && ground_truth.contains(&(i as usize)) {
            has_nearby_truth = true;
            break;
        }
    }

    // If there's a nearby ground truth point and this FP looks like it passed the criteria
    // but the nearby point is the real one, it's suspicious but not a true cliff
    if has_nearby_truth && median_pre > 0.5 && median_post < 0.55 {
        return FPCause::SuspiciousButNotCliff;
    }

    FPCause::Unknown
}

/// Load ground truth from audit.json
/// Returns: (ground_truth, false_positives)
/// - ground_truth: all points with status "Confirmed" (includes both auto-detected and manually injected)
/// - false_positives: all points with status "FalsePositive"
/// Note: excludes points with status "Halftime" from ground truth
fn load_ground_truth(
    audit_path: &std::path::Path,
    _points_path: &std::path::Path,
) -> Result<(HashSet<usize>, HashSet<usize>), StatusCode> {
    let content = fs::read_to_string(audit_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let audit: serde_json::Value = serde_json::from_str(&content)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let cliffs = audit
        .get("cliffs")
        .and_then(|v| v.as_array())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let mut all_confirmed = HashSet::new();
    let mut false_positives = HashSet::new();

    for cliff in cliffs {
        let frame_index = cliff
            .get("frame_index")
            .and_then(|v| v.as_u64())
            .ok_or(StatusCode::BAD_REQUEST)? as usize;

        let status = cliff
            .get("status")
            .and_then(|v| v.as_str())
            .ok_or(StatusCode::BAD_REQUEST)?;

        match status {
            "Confirmed" => {
                all_confirmed.insert(frame_index);
            }
            "FalsePositive" => {
                false_positives.insert(frame_index);
            }
            "Halftime" => {
                // Exclude halftime points from ground truth
            }
            _ => {}
        }
    }

    // Ground truth = confirmed (auto-detected & approved) + injected_fns (manually added)
    // Both are "Confirmed" in audit.json, but we distinguish by presence in points.csv
    let ground_truth = all_confirmed;

    Ok((ground_truth, false_positives))
}

/// Evaluate a detector config against ground truth
/// Ground truth = all points with status "Confirmed" (includes both auto-detected and manually injected)
fn evaluate_config_gpu(
    raw_features: &[FrameFeatures],
    ground_truth: &HashSet<usize>,
    config: &DetectorConfigParams,
    gpu_detector: &Option<Arc<GpuCliffDetector>>,
) -> EvaluationMetrics {
    let detector_config = CliffDetectorConfig::from(config);

    // Recalculate scores with the given field_onset
    let scores: Vec<f32> = raw_features
        .iter()
        .map(|f| {
            recalculate_score_with_field_onset(
                f.left_count,
                f.right_count,
                f.field_count,
                config.field_onset,
            )
        })
        .collect();

    let frames: Vec<(usize, f32)> = raw_features
        .iter()
        .zip(scores.iter())
        .map(|(f, &score)| (f.frame_index, score))
        .collect();

    // Use GPU detector if available, otherwise CPU
    let detected = if let Some(gpu) = gpu_detector {
        match gpu.detect_cliffs(&scores, &detector_config) {
            Ok(cliff_flags) => {
                // Apply min_gap constraint using actual frame indices
                let mut detected = HashSet::new();
                let mut last_cliff_frame_idx: Option<usize> = None;

                for (i, is_cliff) in cliff_flags.iter().enumerate() {
                    if *is_cliff {
                        let frame_idx = frames[i].0;
                        if let Some(last_idx) = last_cliff_frame_idx {
                            if frame_idx - last_idx < config.min_gap {
                                continue;
                            }
                        }
                        detected.insert(frame_idx);
                        last_cliff_frame_idx = Some(frame_idx);
                    }
                }
                detected
            }
            Err(_) => {
                // GPU failed, fall back to CPU stateful detector
                let mut detector_state = CliffDetector::new(detector_config);
                let mut detected = HashSet::new();

                for (frame_idx, score) in &frames {
                    let results = detector_state.push(*frame_idx, *score);
                    for (idx, is_cliff) in results {
                        if is_cliff {
                            detected.insert(idx);
                        }
                    }
                }

                let results = detector_state.flush();
                for (idx, is_cliff) in results {
                    if is_cliff {
                        detected.insert(idx);
                    }
                }
                detected
            }
        }
    } else {
        // No GPU available, use CPU stateful detector
        let mut detector_state = CliffDetector::new(detector_config);
        let mut detected = HashSet::new();

        for (frame_idx, score) in &frames {
            let results = detector_state.push(*frame_idx, *score);
            for (idx, is_cliff) in results {
                if is_cliff {
                    detected.insert(idx);
                }
            }
        }

        let results = detector_state.flush();
        for (idx, is_cliff) in results {
            if is_cliff {
                detected.insert(idx);
            }
        }
        detected
    };

    // Compute metrics
    let tp = ground_truth.intersection(&detected).count();
    let fp = detected.difference(&ground_truth).count();
    let fn_count = ground_truth.difference(&detected).count();

    // Classify false negatives by root cause
    let mut fn_causes: HashMap<String, usize> = HashMap::new();
    for cause in [
        FNCause::FieldSuppression,
        FNCause::LowEzOccupancy,
        FNCause::NoPrePointPlateau,
        FNCause::PlateauButNoDrop,
        FNCause::PostScoreTooHigh,
        FNCause::MinGapSuppressed,
        FNCause::Unknown,
    ] {
        fn_causes.insert(cause.as_str().to_string(), 0);
    }

    let false_negatives: HashSet<usize> = ground_truth.difference(&detected).cloned().collect();
    for fn_idx in false_negatives {
        let cause = classify_fn(fn_idx, &frames, &detected, config);
        *fn_causes.entry(cause.as_str().to_string()).or_insert(0) += 1;
    }

    // Classify false positives by root cause
    let mut fp_causes: HashMap<String, usize> = HashMap::new();
    for cause in [
        FPCause::SuspiciousButNotCliff,
        FPCause::LowQualityData,
        FPCause::IncompleteTransition,
        FPCause::Unknown,
    ] {
        fp_causes.insert(cause.as_str().to_string(), 0);
    }

    let false_positives: HashSet<usize> = detected.difference(&ground_truth).cloned().collect();
    for fp_idx in false_positives {
        let cause = classify_fp(fp_idx, &frames, &ground_truth, config);
        *fp_causes.entry(cause.as_str().to_string()).or_insert(0) += 1;
    }

    let precision = if tp + fp > 0 {
        tp as f64 / (tp + fp) as f64
    } else {
        1.0
    };

    let recall = if tp + fn_count > 0 {
        tp as f64 / (tp + fn_count) as f64
    } else {
        1.0
    };

    let f1 = if precision + recall > 0.0 {
        2.0 * (precision * recall) / (precision + recall)
    } else {
        0.0
    };

    EvaluationMetrics {
        config: config.clone(),
        tp,
        fp,
        fn_count,
        fn_recovered: 0,
        precision,
        recall,
        f1,
        fn_causes,
        fp_causes,
    }
}

/// Evaluate a single detector configuration

/// Generate cartesian product of parameter combinations
fn generate_combinations(
    param_lists: &HashMap<String, Vec<serde_json::Value>>,
) -> Result<Vec<HashMap<String, serde_json::Value>>, StatusCode> {
    if param_lists.is_empty() {
        return Ok(vec![HashMap::new()]);
    }

    let mut keys: Vec<_> = param_lists.keys().collect();
    keys.sort();

    let mut results = vec![HashMap::new()];

    for key in keys {
        let values = &param_lists[key];
        let mut new_results = Vec::new();

        for existing in results {
            for value in values {
                let mut new_combo = existing.clone();
                new_combo.insert(key.to_string(), value.clone());
                new_results.push(new_combo);
            }
        }

        results = new_results;
    }

    Ok(results)
}

/// Sweep parameters across all runs in the output root
pub async fn global_sweep_handler(
    State(state): State<AppState>,
    Json(request): Json<GlobalSweepRequest>,
) -> Result<Json<GlobalSweepResponse>, StatusCode> {
    let output_root = std::path::Path::new(&state.args.output_root);
    let runs = list_runs(output_root).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if runs.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Load data from all runs that have features.csv and audit.json
    let load_start = Instant::now();
    let mut run_data: Vec<(Vec<FrameFeatures>, HashSet<usize>, HashSet<usize>)> = Vec::new();
    let mut csv_load_time = std::time::Duration::ZERO;

    for (run_id, run_context) in &runs {
        // Skip old runs with VID_202601 or VID_202603 (1fps sampling with gaps)
        // Only evaluate on VID_202604+ runs (keyframe-based pipeline)
        if run_id.contains("VID_202601") || run_id.contains("VID_202603") {
            continue;
        }

        let features_path = run_context.output_dir.join("features.csv");
        let audit_path = run_context.output_dir.join("audit.json");
        let points_path = run_context.output_dir.join("points.csv");

        // Skip runs without required files
        if !features_path.exists() || !audit_path.exists() || !points_path.exists() {
            continue;
        }

        let csv_timer = Instant::now();
        if let Ok(raw_features) = load_raw_features(&features_path) {
            csv_load_time += csv_timer.elapsed();
            if let Ok((ground_truth, false_positives)) = load_ground_truth(&audit_path, &points_path) {
                if !ground_truth.is_empty() || !false_positives.is_empty() {
                    run_data.push((raw_features, ground_truth, false_positives));
                }
            }
        } else {
            csv_load_time += csv_timer.elapsed();
        }
    }

    let load_elapsed = load_start.elapsed();
    eprintln!("[PROFILE] Data load: {}ms (CSV: {}ms)", load_elapsed.as_millis(), csv_load_time.as_millis());

    if run_data.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Prepare parameter ranges using loaded detector config
    let mut param_lists = request.ranges.clone();
    let default_params = DetectorConfigParams {
        min_drop: state.detector_config.min_drop,
        min_prepoint_duration: state.detector_config.min_prepoint_duration,
        min_post_duration: state.detector_config.min_post_duration,
        max_post_proba: state.detector_config.max_post_proba,
        absolute_threshold: state.detector_config.absolute_threshold,
        min_gap: state.detector_config.min_gap,
        smoothing_window: state.detector_config.smoothing_window,
        field_onset: state.detector_config.field_onset,
        video_start_prepoint_threshold: state.detector_config.video_start_prepoint_threshold,
    };

    if !param_lists.contains_key("min_drop") {
        param_lists.insert("min_drop".to_string(), vec![json!(default_params.min_drop)]);
    }
    if !param_lists.contains_key("min_prepoint_duration") {
        param_lists.insert("min_prepoint_duration".to_string(), vec![json!(default_params.min_prepoint_duration)]);
    }
    if !param_lists.contains_key("min_post_duration") {
        param_lists.insert("min_post_duration".to_string(), vec![json!(default_params.min_post_duration)]);
    }
    if !param_lists.contains_key("max_post_proba") {
        param_lists.insert("max_post_proba".to_string(), vec![json!(default_params.max_post_proba)]);
    }
    if !param_lists.contains_key("absolute_threshold") {
        param_lists.insert("absolute_threshold".to_string(), vec![json!(default_params.absolute_threshold)]);
    }
    if !param_lists.contains_key("min_gap") {
        param_lists.insert("min_gap".to_string(), vec![json!(default_params.min_gap)]);
    }
    if !param_lists.contains_key("smoothing_window") {
        param_lists.insert("smoothing_window".to_string(), vec![json!(default_params.smoothing_window)]);
    }
    if !param_lists.contains_key("field_onset") {
        param_lists.insert("field_onset".to_string(), vec![json!(default_params.field_onset)]);
    }

    // Generate all combinations
    let combinations = generate_combinations(&param_lists)?;

    // Evaluate baseline across all runs
    let baseline_start = Instant::now();
    let mut baseline_total_tp = 0;
    let mut baseline_total_fp = 0;
    let mut baseline_total_fn = 0;
    let mut baseline_fn_causes: HashMap<String, usize> = HashMap::new();
    let mut baseline_fp_causes: HashMap<String, usize> = HashMap::new();

    for (raw_features, ground_truth, _false_positives) in &run_data {
        let metrics = evaluate_config_gpu(raw_features, ground_truth, &default_params, &state.gpu_detector);
        baseline_total_tp += metrics.tp;
        baseline_total_fp += metrics.fp;
        baseline_total_fn += metrics.fn_count;
        for (cause, count) in metrics.fn_causes {
            *baseline_fn_causes.entry(cause).or_insert(0) += count;
        }
        for (cause, count) in metrics.fp_causes {
            *baseline_fp_causes.entry(cause).or_insert(0) += count;
        }
    }

    let baseline_elapsed = baseline_start.elapsed();

    let baseline_precision = if baseline_total_tp + baseline_total_fp > 0 {
        baseline_total_tp as f64 / (baseline_total_tp + baseline_total_fp) as f64
    } else {
        1.0
    };

    let baseline_recall = if baseline_total_tp + baseline_total_fn > 0 {
        baseline_total_tp as f64 / (baseline_total_tp + baseline_total_fn) as f64
    } else {
        1.0
    };

    let baseline_f1 = if baseline_precision + baseline_recall > 0.0 {
        2.0 * (baseline_precision * baseline_recall) / (baseline_precision + baseline_recall)
    } else {
        0.0
    };

    eprintln!("[PROFILE] Baseline: {}ms | min_pre=10 max_post=0.55 abs_thr=0.50 min_drop=0.15 min_gap=20 | TP={} FP={} FN={} F1={:.3}",
        baseline_elapsed.as_millis(),
        baseline_total_tp,
        baseline_total_fp,
        baseline_total_fn,
        baseline_f1
    );

    let baseline = AggregatedMetrics {
        config: default_params.clone(),
        total_tp: baseline_total_tp,
        total_fp: baseline_total_fp,
        total_fn: baseline_total_fn,
        total_runs_with_data: run_data.len(),
        precision: baseline_precision,
        recall: baseline_recall,
        f1: baseline_f1,
        fn_causes: baseline_fn_causes,
        fp_causes: baseline_fp_causes,
    };

    // Evaluate all combinations (parallelized across configs)
    let sweep_start = Instant::now();
    let num_combinations = combinations.len();

    // Use enumerate to track which config we're on
    let mut results: Vec<AggregatedMetrics> = combinations
        .into_par_iter()
        .enumerate()
        .map(|(combo_idx, combo)| {
            let combo_start = Instant::now();
            let params = DetectorConfigParams {
                min_drop: combo.get("min_drop").and_then(|v| v.as_f64()).unwrap_or(default_params.min_drop as f64) as f32,
                min_prepoint_duration: combo.get("min_prepoint_duration").and_then(|v| v.as_u64()).unwrap_or(default_params.min_prepoint_duration as u64) as usize,
                min_post_duration: combo.get("min_post_duration").and_then(|v| v.as_u64()).unwrap_or(default_params.min_post_duration as u64) as usize,
                max_post_proba: combo.get("max_post_proba").and_then(|v| v.as_f64()).unwrap_or(default_params.max_post_proba as f64) as f32,
                absolute_threshold: combo.get("absolute_threshold").and_then(|v| v.as_f64()).unwrap_or(default_params.absolute_threshold as f64) as f32,
                min_gap: combo.get("min_gap").and_then(|v| v.as_u64()).unwrap_or(default_params.min_gap as u64) as usize,
                smoothing_window: combo.get("smoothing_window").and_then(|v| v.as_u64()).unwrap_or(default_params.smoothing_window as u64) as usize,
                field_onset: combo.get("field_onset").and_then(|v| v.as_f64()).unwrap_or(default_params.field_onset as f64) as f32,
                video_start_prepoint_threshold: combo.get("video_start_prepoint_threshold").and_then(|v| v.as_f64()).unwrap_or(default_params.video_start_prepoint_threshold as f64) as f32,
            };

            let mut total_tp = 0;
            let mut total_fp = 0;
            let mut total_fn = 0;
            let mut fn_causes: HashMap<String, usize> = HashMap::new();
            let mut fp_causes: HashMap<String, usize> = HashMap::new();

            for (raw_features, ground_truth, _false_positives) in &run_data {
                let metrics = evaluate_config_gpu(raw_features, ground_truth, &params, &state.gpu_detector);
                total_tp += metrics.tp;
                total_fp += metrics.fp;
                total_fn += metrics.fn_count;
                for (cause, count) in metrics.fn_causes {
                    *fn_causes.entry(cause).or_insert(0) += count;
                }
                for (cause, count) in metrics.fp_causes {
                    *fp_causes.entry(cause).or_insert(0) += count;
                }
            }

            let precision = if total_tp + total_fp > 0 {
                total_tp as f64 / (total_tp + total_fp) as f64
            } else {
                1.0
            };

            let recall = if total_tp + total_fn > 0 {
                total_tp as f64 / (total_tp + total_fn) as f64
            } else {
                1.0
            };

            let f1 = if precision + recall > 0.0 {
                2.0 * (precision * recall) / (precision + recall)
            } else {
                0.0
            };

            let combo_elapsed = combo_start.elapsed();
            eprintln!("[PROFILE] Config {}/{}: {}ms | min_pre={} max_post={:.2} abs_thr={:.2} min_drop={:.2} min_gap={} | TP={} FP={} FN={} F1={:.3}",
                combo_idx + 1, num_combinations, combo_elapsed.as_millis(),
                params.min_prepoint_duration, params.max_post_proba, params.absolute_threshold, params.min_drop, params.min_gap,
                total_tp, total_fp, total_fn, f1
            );

            AggregatedMetrics {
                config: params,
                total_tp,
                total_fp,
                total_fn,
                total_runs_with_data: run_data.len(),
                precision,
                recall,
                f1,
                fn_causes,
                fp_causes,
            }
        })
        .collect();

    let sweep_elapsed = sweep_start.elapsed();
    eprintln!("[PROFILE] Sweep evaluation ({} configs × {} runs): {}ms total | {}ms avg per config", num_combinations, run_data.len(), sweep_elapsed.as_millis(), sweep_elapsed.as_millis() / num_combinations as u128);

    // Sort by F1 score (descending)
    results.sort_by(|a, b| b.f1.partial_cmp(&a.f1).unwrap_or(std::cmp::Ordering::Equal));

    let total_time = load_elapsed + baseline_elapsed + sweep_elapsed;
    eprintln!("[PROFILE] === TOTAL: {}ms ===", total_time.as_millis());
    eprintln!("[PROFILE] Load: {}% | Baseline: {}% | Sweep: {}%",
        (load_elapsed.as_millis() * 100) / total_time.as_millis(),
        (baseline_elapsed.as_millis() * 100) / total_time.as_millis(),
        (sweep_elapsed.as_millis() * 100) / total_time.as_millis()
    );

    Ok(Json(GlobalSweepResponse {
        results,
        baseline,
        runs_evaluated: run_data.len(),
    }))
}

/// Get the current detector configuration
pub async fn get_detector_config_handler(
    State(state): State<AppState>,
) -> Json<DetectorConfigParams> {
    Json(DetectorConfigParams {
        min_drop: state.detector_config.min_drop,
        min_prepoint_duration: state.detector_config.min_prepoint_duration,
        min_post_duration: state.detector_config.min_post_duration,
        max_post_proba: state.detector_config.max_post_proba,
        absolute_threshold: state.detector_config.absolute_threshold,
        min_gap: state.detector_config.min_gap,
        smoothing_window: state.detector_config.smoothing_window,
        field_onset: state.detector_config.field_onset,
        video_start_prepoint_threshold: state.detector_config.video_start_prepoint_threshold,
    })
}
