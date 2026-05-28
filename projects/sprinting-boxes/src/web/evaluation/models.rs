use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Root causes for false negatives
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FNCause {
    FieldSuppression,
    LowEzOccupancy,
    NoPrePointPlateau,
    PlateauButNoDrop,
    PostScoreTooHigh,
    MinGapSuppressed,
    Unknown,
}

impl FNCause {
    pub fn as_str(&self) -> &'static str {
        match self {
            FNCause::FieldSuppression => "field_suppression",
            FNCause::LowEzOccupancy => "low_ez_occupancy",
            FNCause::NoPrePointPlateau => "no_prepoint_plateau",
            FNCause::PlateauButNoDrop => "plateau_but_no_drop",
            FNCause::PostScoreTooHigh => "post_score_too_high",
            FNCause::MinGapSuppressed => "min_gap_suppressed",
            FNCause::Unknown => "unknown",
        }
    }
}

/// Root causes for false positives
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FPCause {
    SuspiciousButNotCliff,
    LowQualityData,
    IncompleteTransition,
    Unknown,
}

impl FPCause {
    pub fn as_str(&self) -> &'static str {
        match self {
            FPCause::SuspiciousButNotCliff => "suspicious_but_not_cliff",
            FPCause::LowQualityData => "low_quality_data",
            FPCause::IncompleteTransition => "incomplete_transition",
            FPCause::Unknown => "unknown",
        }
    }
}

impl Serialize for FNCause {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FNCause {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "field_suppression" => Ok(FNCause::FieldSuppression),
            "low_ez_occupancy" => Ok(FNCause::LowEzOccupancy),
            "no_prepoint_plateau" => Ok(FNCause::NoPrePointPlateau),
            "plateau_but_no_drop" => Ok(FNCause::PlateauButNoDrop),
            "post_score_too_high" => Ok(FNCause::PostScoreTooHigh),
            "min_gap_suppressed" => Ok(FNCause::MinGapSuppressed),
            _ => Ok(FNCause::Unknown),
        }
    }
}

/// Configuration parameters for the cliff detector
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectorConfigParams {
    pub min_drop: f32,
    pub min_prepoint_duration: usize,
    pub min_post_duration: usize,
    pub max_post_proba: f32,
    pub absolute_threshold: f32,
    pub min_gap: usize,
    pub smoothing_window: usize,
    pub field_onset: f32,
}

impl Default for DetectorConfigParams {
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

/// Results of evaluating a single configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationMetrics {
    pub config: DetectorConfigParams,
    pub tp: usize,
    pub fp: usize,
    pub fn_count: usize,
    pub fn_recovered: usize,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub fn_causes: HashMap<String, usize>,
    pub fp_causes: HashMap<String, usize>,
}

/// Request to sweep across all runs
#[derive(Debug, Deserialize)]
pub struct GlobalSweepRequest {
    /// Map of parameter name to list of values to test
    pub ranges: std::collections::HashMap<String, Vec<serde_json::Value>>,
}

/// Aggregated metrics across all runs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedMetrics {
    pub config: DetectorConfigParams,
    pub total_tp: usize,
    pub total_fp: usize,
    pub total_fn: usize,
    pub total_runs_with_data: usize,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub fn_causes: HashMap<String, usize>,
    pub fp_causes: HashMap<String, usize>,
}

/// Response from global sweep across all runs
#[derive(Debug, Serialize)]
pub struct GlobalSweepResponse {
    pub results: Vec<AggregatedMetrics>,
    pub baseline: AggregatedMetrics,
    pub runs_evaluated: usize,
}
