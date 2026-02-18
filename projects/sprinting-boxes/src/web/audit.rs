use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono;
use opencv::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use crate::cli::Args;
use crate::run_context::list_runs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliffData {
    pub frame_index: usize,
    pub timestamp: String,
    pub left_emptied_first: bool,
    pub right_emptied_first: bool,
    pub maybe_false_positive: bool,
    pub status: String, // "Unconfirmed", "Confirmed", "FalsePositive"
    pub manual_side_override: Option<String>, // "left" or "right"
    pub manual_color_override: Option<String>, // "light" or "dark" (explicit override)
    pub left_team_color: Option<String>, // "light" or "dark" (inferred or overridden)
    pub right_team_color: Option<String>,
    pub score_light: i32,
    pub score_dark: i32,
    pub is_break: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditSettings {
    pub light_team_name: String,
    pub dark_team_name: String,
    pub initial_score_light: i32,
    pub initial_score_dark: i32,
    pub video_start_time: String,
}

impl Default for AuditSettings {
    fn default() -> Self {
        Self {
            light_team_name: "Team A".to_string(),
            dark_team_name: "Team B".to_string(),
            initial_score_light: 0,
            initial_score_dark: 0,
            video_start_time: "00:00:00".to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditState {
    pub cliffs: Vec<CliffData>,
    pub settings: AuditSettings,
}

/// Helper to load audit state, initializing from points.csv if valid
fn load_or_init_audit_state(
    run_context: &crate::run_context::RunContext,
) -> Result<AuditState, StatusCode> {
    let output_dir = &run_context.output_dir;

    // Load points.csv
    let points_path = output_dir.join("points.csv");
    if !points_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let mut cliffs = Vec::new();
    let points_content =
        fs::read_to_string(&points_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Sample rate from run context (fallback to 30.0 if invalid/zero, though unlikely)
    let sample_rate = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        30.0
    };

    for (idx, line) in points_content.lines().enumerate() {
        if idx == 0 {
            continue;
        } // Skip header

        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 4 {
            continue;
        }

        let frame_index: usize = parts[0].parse().unwrap_or(0);
        let left_emptied_first = parts[2].trim() == "1";
        let right_emptied_first = parts[3].trim() == "1";

        cliffs.push(CliffData {
            frame_index,
            timestamp: format_timestamp(frame_index, sample_rate, 0.0),
            left_emptied_first,
            right_emptied_first,
            maybe_false_positive: !left_emptied_first && !right_emptied_first,
            status: "Unconfirmed".to_string(),
            manual_side_override: None,
            manual_color_override: None,
            left_team_color: None,
            right_team_color: None,
            score_light: 0,
            score_dark: 0,
            is_break: false,
        });
    }

    // Load audit.json if it exists (contains user edits)
    let audit_path = output_dir.join("audit.json");
    if audit_path.exists() {
        let audit_content =
            fs::read_to_string(&audit_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let audit_state: AuditState = serde_json::from_str(&audit_content).unwrap_or(AuditState {
            cliffs: cliffs.clone(),
            settings: AuditSettings {
                light_team_name: run_context.light_team_name.clone(),
                dark_team_name: run_context.dark_team_name.clone(),
                ..AuditSettings::default()
            },
        });

        // Merge with loaded cliffs (preserve user edits)
        let mut cliff_map: HashMap<usize, CliffData> = audit_state
            .cliffs
            .into_iter()
            .map(|c| (c.frame_index, c))
            .collect();

        for cliff in cliffs {
            cliff_map.entry(cliff.frame_index).or_insert(cliff);
        }

        let mut merged_cliffs: Vec<CliffData> = cliff_map.into_values().collect();
        merged_cliffs.sort_by_key(|c| c.frame_index);

        // Always recalculate timestamps and scores on load to ensure sync with current settings
        let final_cliffs = recalculate_audit(&merged_cliffs, &audit_state.settings, sample_rate);

        Ok(AuditState {
            cliffs: final_cliffs,
            settings: audit_state.settings,
        })
    } else {
        let settings = AuditSettings {
            light_team_name: run_context.light_team_name.clone(),
            dark_team_name: run_context.dark_team_name.clone(),
            ..AuditSettings::default()
        };
        Ok(AuditState {
            cliffs: recalculate_audit(&cliffs, &settings, sample_rate),
            settings,
        })
    }
}

/// Load cliffs from points.csv and audit.json (if exists)
pub async fn get_cliffs_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
) -> Result<Json<AuditState>, StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = list_runs(output_root).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let audit_state = load_or_init_audit_state(&run_context)?;

    // Sample rate (default 30.0)
    let sample_rate = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        30.0
    };

    // Recalculate scores and breaks
    let enriched_cliffs =
        recalculate_audit(&audit_state.cliffs, &audit_state.settings, sample_rate);

    Ok(Json(AuditState {
        cliffs: enriched_cliffs,
        settings: audit_state.settings,
    }))
}

/// Save audit state (cliffs + settings)
pub async fn save_audit_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
    Json(audit_state): Json<AuditState>,
) -> Result<StatusCode, StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = list_runs(output_root).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let output_dir = &run_context.output_dir;
    let audit_path = output_dir.join("audit.json");

    // Sample rate (default 30.0)
    let sample_rate = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        30.0
    };

    let enriched_cliffs =
        recalculate_audit(&audit_state.cliffs, &audit_state.settings, sample_rate);
    let enriched_state = AuditState {
        cliffs: enriched_cliffs,
        settings: audit_state.settings,
    };

    let json = serde_json::to_string_pretty(&enriched_state)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    fs::write(&audit_path, json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

/// Update audit settings
pub async fn update_audit_settings_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
    Json(settings): Json<AuditSettings>,
) -> Result<StatusCode, StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = list_runs(output_root).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let output_dir = &run_context.output_dir;
    let audit_path = output_dir.join("audit.json");

    let mut audit_state = load_or_init_audit_state(&run_context)?;

    // Sample rate (default 30.0)
    let sample_rate = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        30.0
    };

    audit_state.settings = settings;
    let enriched_cliffs =
        recalculate_audit(&audit_state.cliffs, &audit_state.settings, sample_rate);
    audit_state.cliffs = enriched_cliffs;

    let json = serde_json::to_string_pretty(&audit_state)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    fs::write(&audit_path, json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

/// Update a single cliff field
pub async fn update_cliff_field_handler(
    State(args): State<Arc<Args>>,
    Path((run_id, frame_index, field)): Path<(String, usize, String)>,
) -> Result<StatusCode, StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = list_runs(output_root).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let output_dir = &run_context.output_dir;
    let audit_path = output_dir.join("audit.json");

    let mut audit_state = load_or_init_audit_state(&run_context)?;

    if let Some(cliff) = audit_state
        .cliffs
        .iter_mut()
        .find(|c| c.frame_index == frame_index)
    {
        match field.as_str() {
            "confirm" => cliff.status = "Confirmed".to_string(),
            "reject" => cliff.status = "FalsePositive".to_string(),
            "side" => {
                let current =
                    cliff
                        .manual_side_override
                        .as_deref()
                        .or(if cliff.left_emptied_first {
                            Some("left")
                        } else if cliff.right_emptied_first {
                            Some("right")
                        } else {
                            None
                        });
                cliff.manual_side_override = Some(if current == Some("left") {
                    "right".to_string()
                } else {
                    "left".to_string()
                });
            }
            "colors" => {
                let current_left = cliff
                    .left_team_color
                    .as_deref()
                    .unwrap_or("light")
                    .to_string();

                let new_left = if current_left == "light" {
                    "dark".to_string()
                } else {
                    "light".to_string()
                };

                // Set explicit override for this cliff
                cliff.manual_color_override = Some(new_left);

                // Clear overrides for all subsequent cliffs to force re-inference
                let current_idx = cliff.frame_index;
                for c in audit_state.cliffs.iter_mut() {
                    if c.frame_index > current_idx {
                        c.manual_color_override = None;
                    }
                }
            }
            _ => return Err(StatusCode::BAD_REQUEST),
        }
    } else {
        return Err(StatusCode::NOT_FOUND);
    }

    // Sample rate (default 30.0)
    let sample_rate = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        30.0
    };

    let enriched_cliffs =
        recalculate_audit(&audit_state.cliffs, &audit_state.settings, sample_rate);
    audit_state.cliffs = enriched_cliffs;

    let json = serde_json::to_string_pretty(&audit_state)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    fs::write(&audit_path, json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

fn format_timestamp(frame_index: usize, sample_rate: f64, offset_secs: f64) -> String {
    let total_secs = (frame_index as f64 / sample_rate) + offset_secs;
    let hours = (total_secs / 3600.0) as usize;
    let minutes = ((total_secs % 3600.0) / 60.0) as usize;
    let seconds = (total_secs % 60.0) as usize;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

fn parse_duration_to_secs(duration: &str) -> f64 {
    let parts: Vec<&str> = duration.split(':').collect();
    if parts.len() != 3 {
        return 0.0;
    }
    let h: f64 = parts[0].parse().unwrap_or(0.0);
    let m: f64 = parts[1].parse().unwrap_or(0.0);
    let s: f64 = parts[2].parse().unwrap_or(0.0);
    (h * 3600.0) + (m * 60.0) + s
}

fn recalculate_audit(
    cliffs: &[CliffData],
    settings: &AuditSettings,
    sample_rate: f64,
) -> Vec<CliffData> {
    if cliffs.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut score_light = settings.initial_score_light;
    let mut score_dark = settings.initial_score_dark;
    let mut last_valid_left_color: Option<String> = None;

    let mut sorted_cliffs = cliffs.to_vec();
    sorted_cliffs.sort_by_key(|c| c.frame_index);

    // Pass 1: Core scoring and team assignment
    for (i, cliff) in sorted_cliffs.iter().enumerate() {
        let is_fp = cliff.status == "FalsePositive";

        let total_offset = parse_duration_to_secs(&settings.video_start_time);
        if is_fp {
            result.push(CliffData {
                timestamp: format_timestamp(cliff.frame_index, sample_rate, total_offset),
                left_team_color: None,
                right_team_color: None,
                score_light,
                score_dark,
                is_break: false,
                ..cliff.clone()
            });
            continue;
        }

        // Determine team colors
        // Determine team colors
        let (left, right) = if let Some(ref override_color) = cliff.manual_color_override {
            // Explicit override takes precedence
            let l = override_color.clone();
            let r = if l == "light" {
                "dark".to_string()
            } else {
                "light".to_string()
            };
            (l, r)
        } else if let Some(ref last_color) = last_valid_left_color {
            // Infer from previous (alternating)
            let new_left = if last_color == "light" {
                "dark".to_string()
            } else {
                "light".to_string()
            };
            let new_right = if new_left == "light" {
                "dark".to_string()
            } else {
                "light".to_string()
            };
            (new_left, new_right)
        } else {
            ("light".to_string(), "dark".to_string())
        };

        last_valid_left_color = Some(left.clone());

        // Score update (if not first point)
        if i > 0 {
            let pull_side = cliff
                .manual_side_override
                .as_deref()
                .or(if cliff.left_emptied_first {
                    Some("left")
                } else if cliff.right_emptied_first {
                    Some("right")
                } else {
                    None
                });

            if let Some(side) = pull_side {
                let pulling_team = if side == "left" { &left } else { &right };
                if pulling_team == "light" {
                    score_light += 1;
                } else if pulling_team == "dark" {
                    score_dark += 1;
                }
            }
        }

        result.push(CliffData {
            timestamp: format_timestamp(cliff.frame_index, sample_rate, total_offset),
            left_team_color: Some(left),
            right_team_color: Some(right),
            score_light,
            score_dark,
            is_break: false,
            ..cliff.clone()
        });
    }

    // Pass 2: Break detection
    let valid_points: Vec<CliffData> = result
        .iter()
        .filter(|c| c.status != "FalsePositive")
        .cloned()
        .collect();
    let mut break_indices = Vec::new();

    for j in 0..valid_points.len().saturating_sub(1) {
        let cur = &valid_points[j];
        let next = &valid_points[j + 1];

        let cur_pull_side = cur
            .manual_side_override
            .as_deref()
            .or(if cur.left_emptied_first {
                Some("left")
            } else if cur.right_emptied_first {
                Some("right")
            } else {
                None
            });
        let cur_pull_team = cur_pull_side.and_then(|side| {
            if side == "left" {
                cur.left_team_color.as_ref()
            } else {
                cur.right_team_color.as_ref()
            }
        });

        let next_pull_side = next
            .manual_side_override
            .as_deref()
            .or(if next.left_emptied_first {
                Some("left")
            } else if next.right_emptied_first {
                Some("right")
            } else {
                None
            });
        let next_pull_team = next_pull_side.and_then(|side| {
            if side == "left" {
                next.left_team_color.as_ref()
            } else {
                next.right_team_color.as_ref()
            }
        });

        if let (Some(cur_team), Some(next_team)) = (cur_pull_team, next_pull_team) {
            if cur_team == next_team {
                break_indices.push(cur.frame_index);
            }
        }
    }

    // Apply breaks
    for frame_idx in break_indices {
        if let Some(cliff) = result.iter_mut().find(|c| c.frame_index == frame_idx) {
            cliff.is_break = true;
        }
    }

    result
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureData {
    pub frame_index: usize,
    pub left_count: f32,
    pub right_count: f32,
    pub field_count: f32,
    pub pre_point_score: f32,
    pub crop_path: Option<String>,
    // New features
    pub com_x: Option<f32>,
    pub com_y: Option<f32>,
    pub std_dev: Option<f32>,
    pub com_delta_x: Option<f32>,
    pub com_delta_y: Option<f32>,
    pub std_dev_delta: Option<f32>,
}

pub async fn serve_run_crop_handler(
    State(args): State<Arc<Args>>,
    Path((run_id, filename)): Path<(String, String)>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<impl axum::response::IntoResponse, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = list_runs(output_root).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let crops_dir = run_context.output_dir.join("crops");
    let mut file_path = crops_dir.join(&filename);

    if !file_path.exists() {
        // Fallback: If overview is requested but missing, try to find ANY crop for this frame
        // to support old runs.
        if filename.contains("_overview.jpg") {
            let frame_prefix = filename.split("_overview.jpg").next().unwrap_or("");
            if let Ok(entries) = fs::read_dir(&crops_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with(frame_prefix) && name.ends_with(".jpg") {
                        file_path = crops_dir.join(name);
                        break;
                    }
                }
            }
        }
    }

    if !file_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let final_filename = file_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&filename)
        .to_string();

    // Check if annotation is requested
    let annotate = params.get("annotate").map(|v| v == "true").unwrap_or(false);

    if annotate {
        // Load raw crop image
        let img =
            opencv::imgcodecs::imread(file_path.to_str().unwrap(), opencv::imgcodecs::IMREAD_COLOR)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Parse frame index and suffix from final filename
        let frame_info = final_filename
            .strip_prefix("frame_")
            .and_then(|s| s.strip_suffix(".jpg"))
            .ok_or(StatusCode::BAD_REQUEST)?;

        let parts: Vec<&str> = frame_info.split('_').collect();
        if parts.len() != 2 {
            return Err(StatusCode::BAD_REQUEST);
        }

        let frame_index: usize = parts[0].parse().map_err(|_| StatusCode::BAD_REQUEST)?;
        let suffix = parts[1];

        // Load detections.json to get metadata for this frame
        let detections_path = run_context.output_dir.join("detections.json");
        if !detections_path.exists() {
            tracing::warn!(
                "detections.json missing for run {}, cannot annotate",
                run_id
            );
            return Err(StatusCode::NOT_FOUND);
        }

        let detections_json =
            fs::read_to_string(&detections_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let all_frames: Vec<crate::pipeline::types::DetectedFrame> =
            serde_json::from_str(&detections_json)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Find the matching frame and crop
        let frame = all_frames
            .iter()
            .find(|f| f.id == frame_index)
            .ok_or(StatusCode::NOT_FOUND)?;

        let crop_result = frame
            .results
            .iter()
            .find(|r| r.suffix == suffix)
            .ok_or(StatusCode::NOT_FOUND)?;

        // Draw annotations
        let annotated_img =
            crate::pipeline::finalize::draw_annotations(&img, crop_result, Some(frame))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Encode to JPEG
        let mut buf = opencv::core::Vector::<u8>::new();
        opencv::imgcodecs::imencode(
            ".jpg",
            &annotated_img,
            &mut buf,
            &opencv::core::Vector::new(),
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let data = buf.to_vec();
        let mut response = axum::response::IntoResponse::into_response(data);
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            "image/jpeg".parse().unwrap(),
        );
        Ok(response)
    } else {
        // Serve raw crop
        match fs::read(file_path) {
            Ok(data) => {
                let mut response = axum::response::IntoResponse::into_response(data);
                response.headers_mut().insert(
                    axum::http::header::CONTENT_TYPE,
                    "image/jpeg".parse().unwrap(),
                );
                Ok(response)
            }
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
}

pub async fn get_features_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
) -> Result<Json<Vec<FeatureData>>, StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = list_runs(output_root).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let output_dir = &run_context.output_dir;

    let features_path = output_dir.join("features.csv");
    if !features_path.exists() {
        return Ok(Json(Vec::new()));
    }

    let content =
        fs::read_to_string(&features_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut features = Vec::new();

    for (idx, line) in content.lines().enumerate() {
        if idx == 0 {
            continue;
        } // Skip header

        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 5 {
            continue;
        }

        let frame_index: usize = parts[0].parse().unwrap_or(0);
        let left_count: f32 = parts[1].parse().unwrap_or(0.0);
        let right_count: f32 = parts[2].parse().unwrap_or(0.0);
        let field_count: f32 = parts[3].parse().unwrap_or(0.0);
        let pre_point_score: f32 = parts[4].parse().unwrap_or(0.0);
        // Parse new features if available (backwards compat)
        let com_x = parts
            .get(6)
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|&v| v != -1.0);
        let com_y = parts
            .get(7)
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|&v| v != -1.0);
        let std_dev = parts
            .get(8)
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|&v| v != -1.0);
        let com_delta_x = parts.get(9).and_then(|s| s.parse::<f32>().ok());
        let com_delta_y = parts.get(10).and_then(|s| s.parse::<f32>().ok());
        let std_dev_delta = parts.get(11).and_then(|s| s.parse::<f32>().ok());

        features.push(FeatureData {
            frame_index,
            left_count,
            right_count,
            field_count,
            pre_point_score,
            crop_path: None,
            com_x,
            com_y,
            std_dev,
            com_delta_x,
            com_delta_y,
            std_dev_delta,
        });
    }

    Ok(Json(features))
}

pub async fn get_youtube_chapters_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
) -> Result<String, StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = list_runs(output_root).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let audit_state = load_or_init_audit_state(&run_context)?;

    // Sample rate (default 30.0)
    let sample_rate = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        1.0
    };
    let offset = parse_duration_to_secs(&audit_state.settings.video_start_time);

    let enriched_cliffs =
        recalculate_audit(&audit_state.cliffs, &audit_state.settings, sample_rate);
    let mut chapters = String::new();

    // Always start with 00:00
    chapters.push_str("00:00 Video Start\n");

    for (i, cliff) in enriched_cliffs.iter().enumerate() {
        if cliff.status != "Confirmed" {
            continue;
        }

        let timestamp = format_timestamp(cliff.frame_index, sample_rate, offset);
        let description = get_point_description(cliff, i + 1, &audit_state.settings);
        chapters.push_str(&format!("{} {}\n", timestamp, description));
    }

    Ok(chapters)
}

/// Generates a human-readable description for a point, including team names, score, and whether it was a break.
/// This format is used for both YouTube chapters and Insta360 Studio Clips.
fn get_point_description(
    cliff: &CliffData,
    point_index: usize,
    settings: &AuditSettings,
) -> String {
    let mut description = format!("Point {}", point_index);

    let pull_side = cliff
        .manual_side_override
        .as_deref()
        .or(if cliff.left_emptied_first {
            Some("left")
        } else if cliff.right_emptied_first {
            Some("right")
        } else {
            None
        });

    if let Some(side) = pull_side {
        let team_color = if side == "left" {
            cliff.left_team_color.as_deref().unwrap_or("light")
        } else {
            cliff.right_team_color.as_deref().unwrap_or("dark")
        };

        let team_name = if team_color == "light" {
            &settings.light_team_name
        } else {
            &settings.dark_team_name
        };

        description.push_str(&format!(" - {} Pull", team_name));
    }

    if cliff.is_break {
        description.push_str(" 🔥 Break");
    }

    description.push_str(&format!(
        " ({} {} - {} {})",
        &settings.light_team_name, cliff.score_light, &settings.dark_team_name, cliff.score_dark
    ));

    description
}

/// Handler for GET /api/runs/:id/export/studio-clips
/// Generates an XML file compatible with Insta360 Studio's project/scheme system.
pub async fn get_studio_clips_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let runs = list_runs(std::path::Path::new(&args.output_root))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let audit_state = load_or_init_audit_state(&run_context)?;

    // Use stored metadata
    let total_frames = run_context.total_frames;
    let sample_rate = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        1.0
    };
    let video_fps = run_context.fps;
    let offset = parse_duration_to_secs(&audit_state.settings.video_start_time);
    let total_duration_ms = (((total_frames as f64 / video_fps) + offset) * 1000.0) as u64;

    let now = chrono::Utc::now();
    let utc_now = now.format("%Y.%m.%d %H:%M:%S%.3f").to_string();
    let utc_now_ms = now.timestamp_millis();

    let confirmed_cliffs =
        recalculate_audit(&audit_state.cliffs, &audit_state.settings, sample_rate)
            .into_iter()
            .filter(|c| c.status == "Confirmed")
            .collect::<Vec<_>>();

    let mut point_clips = Vec::new();
    for (i, cliff) in confirmed_cliffs.iter().enumerate() {
        let start_ms = (((cliff.frame_index as f64 / sample_rate) + offset) * 1000.0) as u64;
        let end_ms = if let Some(next) = confirmed_cliffs.get(i + 1) {
            (((next.frame_index as f64 / sample_rate) + offset) * 1000.0) as u64
        } else {
            total_duration_ms
        };
        let description = get_point_description(cliff, i + 1, &audit_state.settings);
        point_clips.push((description, start_ms, end_ms));
    }

    let default_name = if let Some((last_desc, _, _)) = point_clips.last() {
        last_desc.clone()
    } else {
        "Warm-ups".to_string()
    };

    let escaped_default = default_name
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\"", "&quot;")
        .replace("'", "&apos;");

    let mut xml = format!(
        r#"<schemes default="{}">
"#,
        escaped_default
    );

    // 1. Warm-ups clip
    let first_point_ms = if let Some(first) = confirmed_cliffs.first() {
        (((first.frame_index as f64 / sample_rate) + offset) * 1000.0) as u64
    } else {
        total_duration_ms
    };

    xml.push_str(&render_scheme(
        "Warm-ups",
        0,
        first_point_ms,
        total_duration_ms,
        &utc_now,
        utc_now_ms,
    ));

    // 2. Point clips
    for (description, start_ms, end_ms) in point_clips {
        xml.push_str(&render_scheme(
            &description,
            start_ms,
            end_ms,
            total_duration_ms,
            &utc_now,
            utc_now_ms,
        ));
    }

    xml.push_str("</schemes>\n");

    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "application/xml")
        .body(axum::body::Body::from(xml))
        .unwrap())
}

fn render_scheme(
    id: &str,
    start_ms: u64,
    end_ms: u64,
    total_ms: u64,
    utc_now: &str,
    utc_now_ms: i64,
) -> String {
    let escaped_id = id
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\"", "&quot;")
        .replace("'", "&apos;");

    format!(
        r#"    <scheme app_data_id="" app_data_mode="" app_data_ratio="" app_data_source="" app_data_types="" creation="{}" has_deeptrack_user_added="0" has_deeptrack_user_edited="0" has_headtrack_keyframe_user_added="0" has_headtrack_keyframe_user_edited="0" has_keyframe_user_added="0" has_keyframe_user_edited="0" id="{}" last_edit_time="{}" load_hight_data="0">
        <preference duration="{}" favourite="0" last_trim_edit_time="{}" ratio_height="9" ratio_width="16" shell_corrected="0" trim_end="{}" trim_start="{}">
            <bullet_time distance="0.8125" fov="1.0421360963658142"/>
            <rendering accessory="0" ai_raw="0" alpha="0" blend_angle="0" camera_movement="0" cold_shoe="0" cooling_shell="0" deversion="3.5" dewarp="0" dewarp_mode="0" directional_lock="1" distance="0" fov="1.6580628156661987" handle_pano_fpv="0" head_tracking="0" immersion_stab="0" input_data_type="0" motion_blur="0" pano_fpv="0" pano_fpv_horizontal_on="0" pitch="0" projection="64" propeller_guard_on="0" roll="0" rotate_angle="0" stab_direction="0" stab_level="0" stab_type="0" stabilization="1" stabilizer_type_edited="1" use_custom_transform_params="0" yaw="0">
                <play_rate/>
            </rendering>
            <audio denoise_type="0" volume="0.5"/>
            <optimization>
                <calibration offset=""/>
                <stitching ai_stitch="0" audio_mixing="0" audio_mixing_weight="0" audio_mode="9" audio_tracks_default_delay="-2147483648" audio_tracks_delay="-2147483648" beauty_mode="0" color_adjust="1" color_blackpoint_strength="0" color_brightness_strength="0" color_contrast_strength="0" color_definition_strength="0" color_enhancement="0" color_exposure_strength="0" color_highlights_strength="0" color_plus_strength="30" color_saturation_strength="0" color_shadows_strength="0" color_tint_strength="0" color_vibrance_strength="0" color_warmth_strength="0" de_version="3.5" direct_focus="0" dynamic_stitching="0" fade_in_duration_s="5" fade_out_duration_s="3" horizontal_correct="0" horizontal_correct_angle="0" horizontal_correct_default_angle="0" image_fusion="1" is_selfie="0" keyframe_duration_s="5" local_tone_mapping="0" ltm_strength="30" lut="0" motion_blur_edited="0" motion_blur_strength="50" motion_blur_threshold="85" optical_flow_stitching="1" stab_input_data_type="0" templete_stitching="0" under_water_correction="0" under_water_strength="100" under_water_style="0"/>
            </optimization>
            <logo enable_logo="0" enable_time_logo="0" logo_corner_radius="0" logo_feather="0" logo_location="0" logo_rotate="0" logo_size="0.30000001" selected_logo="/panels/none" time_logo_format="1" time_logo_transparency="100"/>
        </preference>
        <timeline app_import_id="" duration_ms="{}">
            <recording>
                <keyframes/>
                <transitions/>
                <headtrackareas/>
                <deep_track_areas/>
            </recording>
            <camera_movement_track/>
        </timeline>
        <dashboard_setting cloud_media="0" cloud_style="" dashboard_datasource_visible="1" dashboard_group_id="" dashboard_group_type="0" dashboard_group_use="1" dashboard_handled="0" dashboard_visible="0"/>
        <pano_animation>
            <configs>
                <config duration="20000" id="1" trim_end="20000" trim_start="0"/>
                <config duration="25000" id="2" trim_end="25000" trim_start="0"/>
                <config duration="20000" id="3" trim_end="20000" trim_start="0"/>
                <config duration="15000" id="4" trim_end="15000" trim_start="0"/>
                <config duration="20000" id="5" trim_end="20000" trim_start="0"/>
            </configs>
        </pano_animation>
    </scheme>
"#,
        utc_now, escaped_id, utc_now, total_ms, utc_now_ms, end_ms, start_ms, total_ms
    )
}

/// Helper to generate M3U playlist content
fn generate_vlc_playlist(args: &Args, run_id: &str) -> Result<String, StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = list_runs(output_root).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == run_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let audit_state = load_or_init_audit_state(&run_context)?;

    // Resolve absolute video path
    let video_root = std::path::Path::new(&args.video_root);
    let video_path = run_context.resolve_video_path(video_root);
    let video_path_str = video_path.to_string_lossy();

    // Sample rate (default 30.0)
    let sample_rate = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        1.0
    };
    let offset = parse_duration_to_secs(&audit_state.settings.video_start_time);

    // Total duration in seconds for the last segment
    let total_frames = run_context.total_frames;
    let video_fps = run_context.fps;
    let total_duration_secs = (total_frames as f64 / video_fps) + offset;

    let confirmed_cliffs =
        recalculate_audit(&audit_state.cliffs, &audit_state.settings, sample_rate)
            .into_iter()
            .filter(|c| c.status == "Confirmed")
            .collect::<Vec<_>>();

    let mut m3u = String::from("#EXTM3U\n");

    for (i, cliff) in confirmed_cliffs.iter().enumerate() {
        let start_time = (cliff.frame_index as f64 / sample_rate) + offset;

        let stop_time = if let Some(next) = confirmed_cliffs.get(i + 1) {
            (next.frame_index as f64 / sample_rate) + offset
        } else {
            total_duration_secs
        };

        let duration = stop_time - start_time;
        let description = get_point_description(cliff, i + 1, &audit_state.settings);

        m3u.push_str(&format!("#EXTVLCOPT:start-time={:.3}\n", start_time));
        m3u.push_str(&format!("#EXTVLCOPT:stop-time={:.3}\n", stop_time));
        m3u.push_str(&format!("#EXTINF:{},{}\n", duration as u64, description));
        m3u.push_str(&format!("{}\n", video_path_str));
    }

    Ok(m3u)
}

/// Handler for GET /api/runs/:id/export/vlc-playlist
pub async fn get_vlc_playlist_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
) -> Result<String, StatusCode> {
    generate_vlc_playlist(&args, &run_id)
}

/// Handler for POST /api/runs/:id/export/vlc-playlist
pub async fn save_vlc_playlist_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let playlist = generate_vlc_playlist(&args, &run_id)?;

    let output_root = std::path::Path::new(&args.output_root);
    let runs = list_runs(output_root).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let output_path = run_context.output_dir.join("playlist.m3u");
    fs::write(output_path, playlist).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}
