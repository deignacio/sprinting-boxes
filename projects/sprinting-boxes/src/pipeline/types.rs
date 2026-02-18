use crate::run_context::CropsConfig;
use opencv::core::Mat;
use serde::{Deserialize, Serialize};
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
    /// Number of active reader workers
    pub active_reader_workers: std::sync::atomic::AtomicUsize,
    /// Number of active crop workers
    pub active_crop_workers: std::sync::atomic::AtomicUsize,
    /// Number of active detection workers
    pub active_detect_workers: std::sync::atomic::AtomicUsize,
    /// Overall processing rate (frames per second)
    pub processing_rate: RwLock<f64>,
    /// Start time of processing
    pub start_time: std::time::Instant,
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
            "feature".to_string(),
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
            active_reader_workers: std::sync::atomic::AtomicUsize::new(0),
            active_crop_workers: std::sync::atomic::AtomicUsize::new(0),
            active_detect_workers: std::sync::atomic::AtomicUsize::new(0),
            processing_rate: RwLock::new(0.0),
            start_time: std::time::Instant::now(),
        }
    }

    pub fn update_stage(&self, stage: &str, delta_count: usize, ms_per_frame: f64) {
        if let Ok(mut stages) = self.stages.write() {
            if let Some(progress) = stages.get_mut(stage) {
                progress.current += delta_count;
                // Simple exponential moving average for smoothing durations
                if progress.ms_per_frame == 0.0 {
                    progress.ms_per_frame = ms_per_frame;
                } else {
                    progress.ms_per_frame = progress.ms_per_frame * 0.9 + ms_per_frame * 0.1;
                }
            }
        }
    }

    /// Update the total number of frames for the run and all stages.
    /// Used when a stage (like reader) finishes early and we discover the actual count.
    pub fn set_total_frames(&self, total: usize) {
        // Update every stage's total to match the new reality in the progress map
        if let Ok(mut stages) = self.stages.write() {
            for progress in stages.values_mut() {
                progress.total = total;
            }
        }
    }

    pub fn to_progress_json(&self) -> serde_json::Value {
        let stages = self.stages.read().unwrap();

        // Calculate effective FPS based on finalized frames
        let finalized = stages.get("finalize").map(|s| s.current).unwrap_or(0);
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let effective_fps = if elapsed > 0.0 {
            finalized as f64 / elapsed
        } else {
            0.0
        };

        // Convert stages to JSON with extra 'fps' field
        let stages_json: BTreeMap<String, serde_json::Value> = stages
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    serde_json::json!({
                        "current": v.current,
                        "total": v.total,
                        "ms_per_frame": v.ms_per_frame,
                        "fps": if v.ms_per_frame > 0.0 { 1000.0 / v.ms_per_frame } else { 0.0 }
                    }),
                )
            })
            .collect();

        serde_json::json!({
            "run_id": self.run_id,
            "total_frames": self.total_frames,
            "is_active": self.is_active.load(Ordering::Relaxed),
            "is_complete": self.is_complete.load(Ordering::Relaxed),
            "error": self.error.read().unwrap().clone(),
            "stages": stages_json,
            "active_reader_workers": self.active_reader_workers.load(Ordering::Relaxed),
            "active_crop_workers": self.active_crop_workers.load(Ordering::Relaxed),
            "active_detect_workers": self.active_detect_workers.load(Ordering::Relaxed),
            "processing_rate": *self.processing_rate.read().unwrap(), // Internal inference rate
            "effective_fps": effective_fps, // Output throughput
            "elapsed_secs": elapsed,
        })
    }
}

/// Configuration for a single regional polygon within a crop
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct RegionalPolygon {
    pub name: String,
    pub polygon: Vec<Point>, // Global or Local based on context
}

/// Configuration for a single crop region (e.g., left endzone, right endzone)
#[derive(Clone)]
pub struct CropConfig {
    pub bbox: BBox,
    pub original_polygon: Vec<Point>,  // Global coords
    pub effective_polygon: Vec<Point>, // Global coords (pre-computed with buffer)
    pub suffix: String,                // e.g., "left", "right", "field", "overview"
    pub regions: Vec<RegionalPolygon>, // NEW: sub-regions for selective detection
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

        vec![CropConfig {
            bbox: convert_bbox(&crops.overview.bbox),
            original_polygon: crops
                .overview
                .original_polygon
                .iter()
                .map(convert_point)
                .collect(),
            effective_polygon: crops
                .overview
                .effective_polygon
                .iter()
                .map(convert_point)
                .collect(),
            suffix: "overview".to_string(),
            regions: vec![
                RegionalPolygon {
                    name: "left".to_string(),
                    polygon: crops
                        .left_end_zone_polygon
                        .iter()
                        .map(convert_point)
                        .collect(),
                },
                RegionalPolygon {
                    name: "right".to_string(),
                    polygon: crops
                        .right_end_zone_polygon
                        .iter()
                        .map(convert_point)
                        .collect(),
                },
                RegionalPolygon {
                    name: "field".to_string(),
                    polygon: crops.field_polygon.iter().map(convert_point).collect(),
                },
            ],
        }]
    }
}

#[derive(Clone)]
pub struct ReaderControl {
    pub range_pool:
        std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<std::ops::Range<usize>>>>,
    pub target_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub tx_v: std::sync::Arc<RwLock<Option<crossbeam::channel::Sender<RawFrame>>>>,
    pub video_path: String,
    pub backend: String,
    pub sample_rate: f64,
}

impl ReaderControl {
    pub fn get_tx(&self) -> Option<crossbeam::channel::Sender<RawFrame>> {
        self.tx_v.read().unwrap().clone()
    }
    pub fn close_tx(&self) {
        self.tx_v.write().unwrap().take();
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
    pub regions: Vec<RegionalPolygon>, // NEW: sub-regions transformed to local coords
}

/// A preprocessed frame containing all crop regions
pub struct PreprocessedFrame {
    pub id: usize,
    pub crops: Vec<CropData>,
}

/// Enriched detection with counting flags
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedDetection {
    pub bbox: BBox, // Corrected to image coords
    pub confidence: f32,
    pub class_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    /// Whether the detection is in an end zone (left/right) - counts towards score
    pub in_end_zone: bool,
    /// Whether the detection is in the field area - counts towards CoM
    pub in_field: bool,
}

/// Result for a single crop region including detections
#[derive(Clone, Serialize, Deserialize)]
pub struct CropResult {
    pub suffix: String,
    pub detections: Vec<EnrichedDetection>,
    pub original_polygon: Vec<Point>,
    pub effective_polygon: Vec<Point>,
    pub bbox: BBox,
    #[serde(skip)]
    pub image: Option<Mat>,
    pub regions: Vec<RegionalPolygon>, // NEW: regions for downstream processing
}

/// A frame after detection has been run
#[derive(Clone, Serialize, Deserialize)]
pub struct DetectedFrame {
    pub id: usize,
    pub results: Vec<CropResult>,
    // Feature fields
    pub left_count: f32,
    pub right_count: f32,
    pub field_count: f32,
    pub pre_point_score: f32,
    pub is_cliff: bool,
    // Heuristic results
    pub left_emptied_first: bool,
    pub right_emptied_first: bool,
    pub maybe_false_positive: bool,
    // CoM and StdDev features
    pub com_x: Option<f32>,
    pub com_y: Option<f32>,
    pub std_dev: Option<f32>,
    pub com_delta_x: Option<f32>,
    pub com_delta_y: Option<f32>,
    pub std_dev_delta: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monotonic_progress() {
        let state = ProcessingState::new("test_run".to_string(), 100);

        // Initial state
        {
            let stages = state.stages.read().unwrap();
            assert_eq!(stages.get("reader").unwrap().current, 0);
            assert_eq!(stages.get("reader").unwrap().total, 100);
        }

        // First update
        state.update_stage("reader", 10, 50.0);
        {
            let stages = state.stages.read().unwrap();
            assert_eq!(stages.get("reader").unwrap().current, 10);
            assert_eq!(stages.get("reader").unwrap().ms_per_frame, 50.0);
        }

        // Second update (concurrent style)
        state.update_stage("reader", 5, 60.0);
        {
            let stages = state.stages.read().unwrap();
            assert_eq!(stages.get("reader").unwrap().current, 15); // Monotonic increment
                                                                   // EMA check: 50 * 0.9 + 60 * 0.1 = 45 + 6 = 51
            assert!((stages.get("reader").unwrap().ms_per_frame - 51.0).abs() < 0.001);
        }
    }
}
