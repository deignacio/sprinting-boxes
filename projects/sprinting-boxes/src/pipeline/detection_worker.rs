use crate::pipeline::detection::ObjectDetector;
use crate::pipeline::slicing::{
    generate_tiles, nms, transform_detection_to_image_coords, SliceConfig,
};
use crate::pipeline::types::{
    BBox, CropResult, DetectedFrame, EnrichedDetection, Point, PreprocessedFrame, ProcessingState,
};
use anyhow::Result;
use crossbeam::channel::{Receiver, Sender};
use opencv::prelude::MatTraitConst;
use std::sync::Arc;
use std::time::Instant;

/// Worker that runs object detection on preprocessed frames.
///
/// It can operate in two modes:
/// 1. Standard detection: runs the model once on the whole crop.
/// 2. Slicing detection: splits the crop into overlapping tiles (SAHI tactic),
///    runs detection on each tile, and merges results using NMS.
pub fn detection_worker(
    rx: Receiver<PreprocessedFrame>,
    tx: Sender<DetectedFrame>,
    model_path: &str,
    min_conf: f32,
    slice_config: SliceConfig,
    state: Arc<ProcessingState>,
    target_count: Arc<std::sync::atomic::AtomicUsize>,
    regions_to_detect: Option<Vec<String>>, // NEW
) -> Result<()> {
    // Load Yolo model
    let mut detector = ObjectDetector::new(model_path)
        .map_err(|e| anyhow::anyhow!("Failed to load model: {}", e))?;
    let slicing_enabled = slice_config.is_enabled();

    for frame in rx {
        // Handle empty/failed frames from upstream by passing through
        if frame.crops.is_empty() {
            tracing::warn!("Detection worker: passing through empty frame {}", frame.id);
        }

        let default_targets = vec!["left".to_string(), "right".to_string(), "field".to_string()];
        let targets = regions_to_detect.as_ref().unwrap_or(&default_targets);
        let start_inst = Instant::now();
        let mut results = Vec::with_capacity(frame.crops.len());

        for crop in frame.crops {
            // Determine regions to detect based on crop suffix and configuration.
            let regions_to_detect_internal = if crop.suffix == "overview" {
                let matched_regions: Vec<_> = crop
                    .regions
                    .iter()
                    .filter(|r| targets.contains(&r.name))
                    .collect();

                if matched_regions.is_empty() {
                    tracing::warn!("No matching regions found for overview crop with targets {:?}. Skipping detection to avoid crash.", targets);
                    results.push(CropResult {
                        suffix: crop.suffix,
                        detections: Vec::new(),
                        original_polygon: crop.original_polygon,
                        effective_polygon: crop.effective_polygon,
                        bbox: BBox {
                            x: 0.0,
                            y: 0.0,
                            w: 1.0,
                            h: 1.0,
                        },
                        image: None,
                        regions: crop.regions,
                    });
                    continue;
                }

                Some(
                    matched_regions
                        .into_iter()
                        .map(|r| r.polygon.clone())
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            };

            let detections = if slicing_enabled {
                detect_with_slicing(
                    &mut detector,
                    &crop.image,
                    &slice_config,
                    min_conf,
                    regions_to_detect_internal.as_deref(),
                )?
            } else {
                detector.detect(&crop.image)?
            };

            let enriched: Vec<EnrichedDetection> = detections
                .into_iter()
                .filter(|d| d.confidence().unwrap_or(0.0) >= min_conf)
                .filter(|d| d.name().unwrap_or("") == "person")
                .map(|d| EnrichedDetection {
                    bbox: BBox {
                        x: d.xmin(),
                        y: d.ymin(),
                        w: d.width(),
                        h: d.height(),
                    },
                    confidence: d.confidence().unwrap_or(0.0),
                    class_id: d.id().unwrap_or(0),
                    class_name: d.name().map(|s| s.to_string()),
                    in_end_zone: false,
                    in_field: false,
                })
                .collect();

            results.push(CropResult {
                suffix: crop.suffix,
                detections: enriched,
                original_polygon: crop.original_polygon,
                effective_polygon: crop.effective_polygon,
                bbox: BBox {
                    x: 0.0,
                    y: 0.0,
                    w: crop.image.cols() as f32,
                    h: crop.image.rows() as f32,
                },
                image: Some(crop.image),
                regions: crop.regions,
            });
        }

        let duration_ms = start_inst.elapsed().as_secs_f64() * 1000.0;
        state.update_stage("detect", 1, duration_ms);

        // Update overall processing rate
        {
            if let Ok(mut rate) = state.processing_rate.write() {
                let current_fps = 1000.0 / duration_ms;
                if *rate == 0.0 {
                    *rate = current_fps;
                } else {
                    *rate = *rate * 0.95 + current_fps * 0.05;
                }
            }
        }

        if tx
            .send(DetectedFrame {
                id: frame.id,
                results,
                left_count: 0.0,
                right_count: 0.0,
                field_count: 0.0,
                pre_point_score: 0.0,
                is_cliff: false,
                left_emptied_first: false,
                right_emptied_first: false,
                maybe_false_positive: false,
                com_x: None,
                com_y: None,
                std_dev: None,
                com_delta_x: None,
                com_delta_y: None,
                std_dev_delta: None,
            })
            .is_err()
        {
            break;
        }

        // Check if we should exit after processing
        let current_target = target_count.load(std::sync::atomic::Ordering::Relaxed);
        let current_active = state
            .active_detect_workers
            .load(std::sync::atomic::Ordering::Relaxed);
        if current_active > current_target {
            tracing::info!("Detection worker exiting to scale down");
            break;
        }
    }

    Ok(())
}

/// Runs inference using a sliding window (slicing) strategy to detect small objects.
fn detect_with_slicing(
    detector: &mut ObjectDetector,
    image: &opencv::core::Mat,
    config: &SliceConfig,
    min_conf: f32,
    regions: Option<&[Vec<Point>]>,
) -> Result<Vec<usls::Hbb>> {
    let tiles = generate_tiles(image, config, regions)?;

    if tiles.is_empty() {
        return detector.detect(image);
    }
    tracing::debug!("Detecting with slicing: {} tiles", tiles.len());

    let tile_images: Vec<opencv::core::Mat> = tiles.iter().map(|t| t.image.clone()).collect();
    let batch_results = detector.detect_batch(&tile_images)?;

    let mut all_detections = Vec::new();
    for (tile, detections) in tiles.iter().zip(batch_results) {
        for det in detections {
            if det.confidence().unwrap_or(0.0) < min_conf {
                continue;
            }
            let transformed = transform_detection_to_image_coords(&det, tile);
            all_detections.push(transformed);
        }
    }

    Ok(nms(all_detections, config.nms_iou_threshold))
}
