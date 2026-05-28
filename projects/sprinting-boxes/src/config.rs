use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::scoring::CliffDetectorConfig;
use crate::web::evaluation::models::DetectorConfigParams;

/// Configuration for the cliff detector, loaded from detector.config.yaml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectorConfig {
    pub min_drop: f32,
    pub min_prepoint_duration: usize,
    pub min_post_duration: usize,
    pub max_post_proba: f32,
    pub absolute_threshold: f32,
    pub min_gap: usize,
    pub smoothing_window: usize,
    pub field_onset: f32,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            min_drop: 0.15,
            min_prepoint_duration: 10,
            min_post_duration: 10,
            max_post_proba: 0.55,
            absolute_threshold: 0.5,
            min_gap: 20,
            smoothing_window: 3,
            field_onset: 1.5,
        }
    }
}

impl DetectorConfig {
    /// Load configuration from a YAML file.
    /// Falls back to defaults if file doesn't exist or fails to parse.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Self {
        let path = path.as_ref();

        match fs::read_to_string(path) {
            Ok(contents) => {
                match serde_yaml::from_str::<Self>(&contents) {
                    Ok(config) => {
                        eprintln!("[CONFIG] Loaded detector config from {}", path.display());
                        config
                    }
                    Err(e) => {
                        eprintln!("[CONFIG] Failed to parse config file {}: {}", path.display(), e);
                        eprintln!("[CONFIG] Using default configuration");
                        Self::default()
                    }
                }
            }
            Err(e) => {
                eprintln!("[CONFIG] Failed to read config file {}: {}", path.display(), e);
                eprintln!("[CONFIG] Using default configuration");
                Self::default()
            }
        }
    }
}

impl From<DetectorConfig> for CliffDetectorConfig {
    fn from(config: DetectorConfig) -> Self {
        Self {
            min_drop: config.min_drop,
            min_prepoint_duration: config.min_prepoint_duration,
            min_post_duration: config.min_post_duration,
            max_post_proba: config.max_post_proba,
            absolute_threshold: config.absolute_threshold,
            min_gap: config.min_gap,
            smoothing_window: config.smoothing_window,
        }
    }
}

impl From<&DetectorConfigParams> for CliffDetectorConfig {
    fn from(config: &DetectorConfigParams) -> Self {
        Self {
            min_drop: config.min_drop,
            min_prepoint_duration: config.min_prepoint_duration,
            min_post_duration: config.min_post_duration,
            max_post_proba: config.max_post_proba,
            absolute_threshold: config.absolute_threshold,
            min_gap: config.min_gap,
            smoothing_window: config.smoothing_window,
        }
    }
}

impl From<&DetectorConfig> for DetectorConfigParams {
    fn from(config: &DetectorConfig) -> Self {
        Self {
            min_drop: config.min_drop,
            min_prepoint_duration: config.min_prepoint_duration,
            min_post_duration: config.min_post_duration,
            max_post_proba: config.max_post_proba,
            absolute_threshold: config.absolute_threshold,
            min_gap: config.min_gap,
            smoothing_window: config.smoothing_window,
            field_onset: config.field_onset,
        }
    }
}
