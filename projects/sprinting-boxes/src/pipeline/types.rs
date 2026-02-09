pub use crate::run_artifacts::{BBox, Point};
use crate::run_context::CropsConfig;
use opencv::core::Mat;
use serde_json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::RwLock;

/// Processing state shared between workers and SSE handler
#[derive(Debug)]
pub struct ProcessingState {
    pub run_id: String,
    pub frames_read: AtomicUsize,
    pub frames_processed: AtomicUsize,
    pub total_frames: AtomicUsize,
    pub is_active: AtomicBool,
    pub is_complete: AtomicBool,
    pub error: RwLock<Option<String>>,
}

impl ProcessingState {
    pub fn new(run_id: String, total_frames: usize) -> Self {
        Self {
            run_id,
            frames_read: AtomicUsize::new(0),
            frames_processed: AtomicUsize::new(0),
            total_frames: AtomicUsize::new(total_frames),
            is_active: AtomicBool::new(true),
            is_complete: AtomicBool::new(false),
            error: RwLock::new(None),
        }
    }

    pub fn to_progress_json(&self) -> serde_json::Value {
        serde_json::json!({
            "run_id": self.run_id,
            "frames_read": self.frames_read.load(Ordering::Relaxed),
            "frames_processed": self.frames_processed.load(Ordering::Relaxed),
            "total_frames": self.total_frames.load(Ordering::Relaxed),
            "is_active": self.is_active.load(Ordering::Relaxed),
            "is_complete": self.is_complete.load(Ordering::Relaxed),
            "error": self.error.read().unwrap().clone(),
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
pub struct CropData {
    pub image: Mat,
    pub original_polygon: Option<Vec<Point>>, // Local crop coords
    pub effective_polygon: Option<Vec<Point>>, // Local crop coords
    pub suffix: String,
}

/// A preprocessed frame containing all crop regions
pub struct PreprocessedFrame {
    pub id: usize,
    pub crops: Vec<CropData>,
}
