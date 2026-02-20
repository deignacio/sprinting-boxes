// Audit utility functions for score calculation and timestamp formatting
//
// This module contains the core business logic for:
// - Timestamp formatting from frame indices
// - Duration parsing
// - Score recalculation with team assignment and break detection

use super::models::{AuditSettings, CliffData};

/// Format a timestamp from a frame index, sample rate, and offset
pub fn format_timestamp(frame_index: usize, sample_rate: f64, offset_secs: f64) -> String {
    let total_secs = (frame_index as f64 / sample_rate) + offset_secs;
    let hours = (total_secs / 3600.0) as usize;
    let minutes = ((total_secs % 3600.0) / 60.0) as usize;
    let seconds = (total_secs % 60.0) as usize;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

/// Parse a duration string in HH:MM:SS format to seconds
pub fn parse_duration_to_secs(duration: &str) -> f64 {
    let parts: Vec<&str> = duration.split(':').collect();
    if parts.len() != 3 {
        return 0.0;
    }
    let h: f64 = parts[0].parse().unwrap_or(0.0);
    let m: f64 = parts[1].parse().unwrap_or(0.0);
    let s: f64 = parts[2].parse().unwrap_or(0.0);
    (h * 3600.0) + (m * 60.0) + s
}

/// Get the sample rate from a run context, with a default fallback
pub fn get_sample_rate(sample_rate: f64) -> f64 {
    if sample_rate > 0.0 {
        sample_rate
    } else {
        30.0
    }
}

/// Recalculate audit state: scores, team colors, and breaks
///
/// This is the core business logic for the audit system. It:
/// 1. Sorts cliffs by frame index
/// 2. Assigns team colors (alternating light/dark)
/// 3. Calculates scores based on pull side
/// 4. Detects breaks (same team pulls twice in a row)
pub fn recalculate_audit(
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
    let mut valid_point_count = 0;

    let mut sorted_cliffs = cliffs.to_vec();
    sorted_cliffs.sort_by_key(|c| c.frame_index);

    let total_offset = parse_duration_to_secs(&settings.video_start_time);

    // Pass 1: Core scoring and team assignment
    for cliff in sorted_cliffs.iter() {
        let is_fp = cliff.status == "FalsePositive";

        if cliff.status == "Halftime" {
            if cliff.halftime_winner.as_deref() == Some("light") {
                score_light += 1;
            } else if cliff.halftime_winner.as_deref() == Some("dark") {
                score_dark += 1;
            }

            valid_point_count = 0; // Reset for second half

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
        if valid_point_count > 0 {
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

        valid_point_count += 1;

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
