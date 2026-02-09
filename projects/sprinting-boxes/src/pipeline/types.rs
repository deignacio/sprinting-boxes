use crate::run_context::CropsConfig;
use opencv::core::Mat;
use serde::Serialize;
use serde_json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

pub use crate::run_artifacts::{BBox, Point};

#[derive(Debug, Serialize, Clone)]
pub struct StageProgress {
    pub current: usize,
    pub total: usize,
    pub ms_per_frame: f64,
}

/// Processing state shared between workers and SSE handler
#[derive(Debug)]
pub struct ProcessingState {
    pub run_id: String,
    pub total_frames: usize,
    pub is_active: AtomicBool,
    pub is_complete: AtomicBool,
    pub error: RwLock<Option<String>>,
    /// Progress per stage (e.g., "reader", "crop", "detect", "finalize")
    pub stages: RwLock<BTreeMap<String, StageProgress>>,
}

impl ProcessingState {
    pub fn new(run_id: String, total_frames: usize) -> Self {
        let mut stages = BTreeMap::new();
        // Initialize stages
        stages.insert(
            "reader".to_string(),
            StageProgress {
                current: 0,
                total: total_frames,
                ms_per_frame: 0.0,
            },
        );
        stages.insert(
            "crop".to_string(),
            StageProgress {
                current: 0,
                total: total_frames,
                ms_per_frame: 0.0,
            },
        );
        stages.insert(
            "detect".to_string(),
            StageProgress {
                current: 0,
                total: total_frames,
                ms_per_frame: 0.0,
            },
        );
        stages.insert(
            "finalize".to_string(),
            StageProgress {
                current: 0,
                total: total_frames,
                ms_per_frame: 0.0,
            },
        );

        Self {
            run_id,
            total_frames,
            is_active: AtomicBool::new(true),
            is_complete: AtomicBool::new(false),
            error: RwLock::new(None),
            stages: RwLock::new(stages),
        }
    }

    pub fn update_stage(&self, stage: &str, current: usize, ms_per_frame: f64) {
        if let Ok(mut stages) = self.stages.write() {
            if let Some(progress) = stages.get_mut(stage) {
                progress.current = current;
                // Simple exponential moving average for smoothing durations
                if progress.ms_per_frame == 0.0 {
                    progress.ms_per_frame = ms_per_frame;
                } else {
                    progress.ms_per_frame = progress.ms_per_frame * 0.9 + ms_per_frame * 0.1;
                }
            }
        }
    }

    pub fn to_progress_json(&self) -> serde_json::Value {
        serde_json::json!({
            "run_id": self.run_id,
            "total_frames": self.total_frames,
            "is_active": self.is_active.load(Ordering::Relaxed),
            "is_complete": self.is_complete.load(Ordering::Relaxed),
            "error": self.error.read().unwrap().clone(),
            "stages": *self.stages.read().unwrap(),
        })
    }
}

/// Configuration for a single crop region (e.g., left endzone, right endzone)
#[derive(Clone)]
pub struct CropConfig {
    pub bbox: BBox,
    pub original_polygon: Vec<Point>,  // Global coords
    pub effective_polygon: Vec<Point>, // Global coords (pre-computed with buffer)
    pub suffix: String,                // e.g., "left", "right", "field"
}

impl From<&CropsConfig> for Vec<CropConfig> {
    fn from(crops: &CropsConfig) -> Self {
        let convert_point = |p: &crate::run_context::Point| Point { x: p.x, y: p.y };
        let convert_bbox = |b: &crate::run_context::BBox| BBox {
            x: b.x,
            y: b.y,
            w: b.w,
            h: b.h,
        };

        vec![
            CropConfig {
                bbox: convert_bbox(&crops.left_end_zone.bbox),
                original_polygon: crops
                    .left_end_zone
                    .original_polygon
                    .iter()
                    .map(convert_point)
                    .collect(),
                effective_polygon: crops
                    .left_end_zone
                    .effective_polygon
                    .iter()
                    .map(convert_point)
                    .collect(),
                suffix: "left".to_string(),
            },
            CropConfig {
                bbox: convert_bbox(&crops.right_end_zone.bbox),
                original_polygon: crops
                    .right_end_zone
                    .original_polygon
                    .iter()
                    .map(convert_point)
                    .collect(),
                effective_polygon: crops
                    .right_end_zone
                    .effective_polygon
                    .iter()
                    .map(convert_point)
                    .collect(),
                suffix: "right".to_string(),
            },
        ]
    }
}

/// A raw frame read from the video source
pub struct RawFrame {
    pub id: usize,
    pub mat: Mat,
}

/// Data for a single cropped region
#[derive(Clone)]
pub struct CropData {
    pub image: Mat,
    pub original_polygon: Vec<Point>,  // Local crop coords
    pub effective_polygon: Vec<Point>, // Local crop coords
    pub suffix: String,
}

/// A preprocessed frame containing all crop regions
pub struct PreprocessedFrame {
    pub id: usize,
    pub crops: Vec<CropData>,
}

/// Enriched detection with counting flags
#[derive(Debug, Clone, Serialize)]
pub struct EnrichedDetection {
    pub bbox: BBox, // Corrected to image coords
    pub confidence: f32,
    pub class_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    pub is_counted: bool,
}

/// Result for a single crop region including detections
#[derive(Clone, Serialize)]
pub struct CropResult {
    pub suffix: String,
    pub detections: Vec<EnrichedDetection>,
    pub original_polygon: Vec<Point>,
    pub effective_polygon: Vec<Point>,
    pub bbox: BBox,
    #[serde(skip)]
    pub image: Option<Mat>,
}

/// A frame after detection has been run
#[derive(Clone, Serialize)]
pub struct DetectedFrame {
    pub id: usize,
    pub results: Vec<CropResult>,
}
