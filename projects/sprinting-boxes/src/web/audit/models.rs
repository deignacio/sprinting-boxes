use serde::{Deserialize, Serialize};

/// Data structure representing a detected cliff event in the video
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliffData {
    pub frame_index: usize,
    pub timestamp: String,
    pub left_emptied_first: bool,
    pub right_emptied_first: bool,
    pub maybe_false_positive: bool,
    pub status: String, // "Unconfirmed", "Confirmed", "FalsePositive", "Halftime"
    pub halftime_winner: Option<String>, // "light" or "dark"
    pub manual_side_override: Option<String>, // "left" or "right"
    pub manual_color_override: Option<String>, // "light" or "dark" (explicit override)
    pub left_team_color: Option<String>, // "light" or "dark" (inferred or overridden)
    pub right_team_color: Option<String>,
    pub score_light: i32,
    pub score_dark: i32,
    pub is_break: bool,
}

/// Settings for the audit system, including team names and initial scores
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

/// The complete audit state including all cliffs and settings
#[derive(Debug, Serialize, Deserialize)]
pub struct AuditState {
    pub cliffs: Vec<CliffData>,
    pub settings: AuditSettings,
}

impl CliffData {
    /// Create a new unconfirmed cliff from raw detection data
    #[allow(dead_code)]
    pub fn from_detection(
        frame_index: usize,
        timestamp: String,
        left_emptied_first: bool,
        right_emptied_first: bool,
    ) -> Self {
        Self {
            frame_index,
            timestamp,
            left_emptied_first,
            right_emptied_first,
            maybe_false_positive: !left_emptied_first && !right_emptied_first,
            status: "Unconfirmed".to_string(),
            halftime_winner: None,
            manual_side_override: None,
            manual_color_override: None,
            left_team_color: None,
            right_team_color: None,
            score_light: 0,
            score_dark: 0,
            is_break: false,
        }
    }
}
