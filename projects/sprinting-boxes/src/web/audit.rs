use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
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
    pub time_offset_secs: f64,
    pub video_start_time: String,
}

impl Default for AuditSettings {
    fn default() -> Self {
        Self {
            light_team_name: "Team A".to_string(),
            dark_team_name: "Team B".to_string(),
            initial_score_light: 0,
            initial_score_dark: 0,
            time_offset_secs: 0.0,
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
    let fps = if run_context.sample_rate > 0.0 {
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
            timestamp: format_timestamp(frame_index, fps, 0.0),
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
        let final_cliffs = recalculate_audit(&merged_cliffs, &audit_state.settings, fps);

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
            cliffs: recalculate_audit(&cliffs, &settings, fps),
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
    let fps = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        30.0
    };

    // Recalculate scores and breaks
    let enriched_cliffs = recalculate_audit(&audit_state.cliffs, &audit_state.settings, fps);

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
    let fps = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        30.0
    };

    let enriched_cliffs = recalculate_audit(&audit_state.cliffs, &audit_state.settings, fps);
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
    let fps = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        30.0
    };

    audit_state.settings = settings;
    let enriched_cliffs = recalculate_audit(&audit_state.cliffs, &audit_state.settings, fps);
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
    let fps = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        30.0
    };

    let enriched_cliffs = recalculate_audit(&audit_state.cliffs, &audit_state.settings, fps);
    audit_state.cliffs = enriched_cliffs;

    let json = serde_json::to_string_pretty(&audit_state)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    fs::write(&audit_path, json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

fn format_timestamp(frame_index: usize, fps: f64, offset_secs: f64) -> String {
    let total_secs = (frame_index as f64 / fps) + offset_secs;
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

fn recalculate_audit(cliffs: &[CliffData], settings: &AuditSettings, fps: f64) -> Vec<CliffData> {
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

        let total_offset =
            settings.time_offset_secs + parse_duration_to_secs(&settings.video_start_time);
        if is_fp {
            result.push(CliffData {
                timestamp: format_timestamp(cliff.frame_index, fps, total_offset),
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
            timestamp: format_timestamp(cliff.frame_index, fps, total_offset),
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
}

pub async fn serve_run_crop_handler(
    State(args): State<Arc<Args>>,
    Path((run_id, filename)): Path<(String, String)>,
) -> Result<impl axum::response::IntoResponse, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = list_runs(output_root).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let crops_dir = run_context.output_dir.join("crops");
    let file_path = crops_dir.join(filename);

    if !file_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

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

        features.push(FeatureData {
            frame_index,
            left_count,
            right_count,
            field_count,
            pre_point_score,
            crop_path: None,
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
    let fps = if run_context.sample_rate > 0.0 {
        run_context.sample_rate
    } else {
        30.0
    };
    let offset = audit_state.settings.time_offset_secs
        + parse_duration_to_secs(&audit_state.settings.video_start_time);

    let enriched_cliffs = recalculate_audit(&audit_state.cliffs, &audit_state.settings, fps);
    let mut chapters = String::new();

    // Always start with 00:00
    chapters.push_str("00:00 Video Start\n");

    for (i, cliff) in enriched_cliffs.iter().enumerate() {
        if cliff.status != "Confirmed" {
            continue;
        }

        let timestamp = format_timestamp(cliff.frame_index, fps, offset);

        let mut chapter_line = format!("{} Point {}", timestamp, i + 1);

        // Determine pulling team name
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
                &audit_state.settings.light_team_name
            } else {
                &audit_state.settings.dark_team_name
            };

            chapter_line.push_str(&format!(" - {} Pull", team_name));
        }

        if cliff.is_break {
            chapter_line.push_str(" 🔥 Break");
        }

        // Add Score info
        chapter_line.push_str(&format!(
            " ({} {} - {} {})",
            &audit_state.settings.light_team_name,
            cliff.score_light,
            &audit_state.settings.dark_team_name,
            cliff.score_dark
        ));

        chapters.push_str(&chapter_line);
        chapters.push('\n');
    }

    Ok(chapters)
}
