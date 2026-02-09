use crate::pipeline::detection::ObjectDetector;
use crate::pipeline::slicing::{
    generate_tiles, nms, transform_detection_to_image_coords, SliceConfig,
};
use crate::pipeline::types::{
    BBox, CropResult, DetectedFrame, EnrichedDetection, PreprocessedFrame, ProcessingState,
};
use anyhow::Result;
use crossbeam::channel::{Receiver, Sender};
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
) -> Result<()> {
    // Initialize detector
    let mut detector = ObjectDetector::new(model_path)?;
    let slicing_enabled = slice_config.is_enabled();

    for frame in rx {
        let start_inst = Instant::now();
        let mut results = Vec::with_capacity(frame.crops.len());

        for crop in frame.crops {
            let detections = if slicing_enabled {
                detect_with_slicing(&mut detector, &crop.image, &slice_config, min_conf)?
            } else {
                detector.detect(&crop.image)?
            };

            let enriched: Vec<EnrichedDetection> = detections
                .into_iter()
                .filter(|d| d.confidence().unwrap_or(0.0) >= min_conf)
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
                    is_counted: false,
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
                    w: 1.0,
                    h: 1.0,
                }, // placeholder, will be set by orchestrator if needed
                image: Some(crop.image),
            });
        }

        let duration_ms = start_inst.elapsed().as_secs_f64() * 1000.0;
        state.update_stage("detect", frame.id, duration_ms);

        if tx
            .send(DetectedFrame {
                id: frame.id,
                results,
            })
            .is_err()
        {
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
) -> Result<Vec<usls::Hbb>> {
    let tiles = generate_tiles(image, config)?;

    if tiles.is_empty() {
        return detector.detect(image);
    }

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
