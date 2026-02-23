use crate::pipeline::detection::ObjectDetector;
use crate::pipeline::slicing::{
    generate_tiles, nms, transform_detection_to_image_coords, HbbWrapper, SliceConfig,
};
use crate::pipeline::types::{
    BBox, CropResult, DetectedFrame, DetectionSummary, EnrichedDetection, PreprocessedFrame,
    ProcessingState,
};
use anyhow::Result;
use crossbeam::channel::{Receiver, Sender};
use opencv::prelude::MatTraitConst;
use std::sync::Arc;
use std::time::Instant;

/// Parameters for the detection worker to avoid too many arguments clippy warning.
pub struct DetectionParams {
    pub model_path: String,
    pub min_conf: f32,
    pub slice_config: SliceConfig,
    pub regions_to_detect: Option<Vec<String>>,
}

pub fn detection_worker(
    rx: Receiver<PreprocessedFrame>,
    tx: Sender<DetectedFrame>,
    params: DetectionParams,
    state: Arc<ProcessingState>,
    target_count: Arc<std::sync::atomic::AtomicUsize>,
) -> Result<()> {
    // Load Yolo model
    let mut detector = ObjectDetector::new(&params.model_path)
        .map_err(|e| anyhow::anyhow!("Failed to load model: {}", e))?;
    let slicing_enabled = params.slice_config.is_enabled();
    tracing::info!(
        "Detection worker started with slice_config: {:?}",
        params.slice_config
    );

    for frame in rx {
        // Handle empty/failed frames from upstream by passing through
        if frame.crops.is_empty() {
            tracing::warn!("Detection worker: passing through empty frame {}", frame.id);
        }

        let default_targets = vec!["left".to_string(), "right".to_string(), "field".to_string()];
        let targets = params
            .regions_to_detect
            .as_ref()
            .unwrap_or(&default_targets);
        let start_inst = Instant::now();

        // 1. Tile Generation Phase: Collect tiles from all crops
        struct QueuedTile {
            crop_index: usize,
            tile: crate::pipeline::slicing::Tile,
        }
        let mut all_queued_tiles = Vec::new();

        for (crop_index, crop) in frame.crops.iter().enumerate() {
            let regions_to_tile = if crop.suffix == "overview" {
                // Overview: only process if explicitly requested
                if !targets.contains(&"overview".to_string()) {
                    continue;
                }

                // Field mode usually: only tile regions that are in targets (like 'field')
                let matched_regions: Vec<_> = crop
                    .regions
                    .iter()
                    .filter(|r| targets.contains(&r.name))
                    .collect();

                if matched_regions.is_empty() {
                    continue;
                }

                Some(
                    matched_regions
                        .into_iter()
                        .map(|r| r.polygon.clone())
                        .collect::<Vec<_>>(),
                )
            } else {
                // EZ crops (left/right): skip if not in targets
                if !targets.contains(&crop.suffix) {
                    continue;
                }
                None
            };

            if slicing_enabled {
                let tiles = generate_tiles(
                    &crop.image,
                    &params.slice_config,
                    regions_to_tile.as_deref(),
                )?;
                for tile in tiles {
                    all_queued_tiles.push(QueuedTile { crop_index, tile });
                }
            } else {
                // Standard detection: one "fake" tile covering the whole image
                all_queued_tiles.push(QueuedTile {
                    crop_index,
                    tile: crate::pipeline::slicing::Tile {
                        image: crop.image.clone(),
                        x_offset: 0,
                        y_offset: 0,
                        original_width: crop.image.cols(),
                        original_height: crop.image.rows(),
                    },
                });
            }
        }

        tracing::debug!(
            "Detection worker: {} total tiles for frame {}",
            all_queued_tiles.len(),
            frame.id
        );

        // 2. Inference Phase: Batch detect all collected tiles
        let mut detections_by_crop = vec![Vec::new(); frame.crops.len()];
        if !all_queued_tiles.is_empty() {
            let tile_images: Vec<opencv::core::Mat> = all_queued_tiles
                .iter()
                .map(|t| t.tile.image.clone())
                .collect();
            let chunk_size = safe_chunk_size(all_queued_tiles.len());
            let batch_results = detector.detect_batch(&tile_images, chunk_size)?;

            for (queued, detections) in all_queued_tiles.into_iter().zip(batch_results) {
                for det in detections {
                    if det.confidence().unwrap_or(0.0) < params.min_conf {
                        continue;
                    }
                    if det.name().unwrap_or("") != "person" {
                        continue;
                    }

                    // Transform detection back to crop pixel coordinates
                    let transformed = transform_detection_to_image_coords(&det, &queued.tile);
                    detections_by_crop[queued.crop_index].push(transformed);
                }
            }
        }

        // 3. Re-assembly Phase: Create CropResults and merge EZ detections into overview
        let mut results = Vec::with_capacity(frame.crops.len());
        let overview_info: Option<(usize, BBox, f32, f32)> =
            frame.crops.iter().enumerate().find_map(|(i, c)| {
                if c.suffix == "overview" {
                    let size = c.image.size().ok()?;
                    Some((i, c.source_bbox, size.width as f32, size.height as f32))
                } else {
                    None
                }
            });

        // Initialize CropResults and track NMS statistics
        let mut nms_stats_by_crop = Vec::new();
        for (i, crop) in frame.crops.iter().enumerate() {
            // Convert usls::Hbb to HbbWrapper for unified NMS
            let wrapped_detections: Vec<HbbWrapper> = detections_by_crop[i]
                .clone()
                .into_iter()
                .map(HbbWrapper::from)
                .collect();

            let (nms_results, nms_stat) =
                nms(wrapped_detections, params.slice_config.nms_iou_threshold);
            nms_stats_by_crop.push((crop.suffix.clone(), nms_stat));

            // Convert back from HbbWrapper to usls::Hbb
            let nms_hbbs: Vec<usls::Hbb> =
                nms_results.into_iter().map(|wrapper| wrapper.0).collect();

            let size = crop.image.size()?;

            results.push(CropResult {
                suffix: crop.suffix.clone(),
                detections: nms_hbbs
                    .into_iter()
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
                        in_end_zone: crop.suffix == "left" || crop.suffix == "right",
                        in_field: crop.suffix == "overview", // Initial guess, will be refined in feature.rs
                    })
                    .collect(),
                original_polygon: crop.original_polygon.clone(),
                effective_polygon: crop.effective_polygon.clone(),
                bbox: BBox {
                    x: 0.0,
                    y: 0.0,
                    w: size.width as f32,
                    h: size.height as f32,
                },
                image: Some(crop.image.clone()),
                regions: crop.regions.clone(),
            });
        }

        // 4. Merging Phase: Merge EZ detections into the overview CropResult
        let mut merge_nms_stat = None;
        if let Some((ov_index, ov_bbox, ov_w, ov_h)) = overview_info {
            let mut merged_ez_detections = Vec::new();

            for (i, crop) in frame.crops.iter().enumerate() {
                if i == ov_index {
                    continue;
                }
                if crop.suffix != "left" && crop.suffix != "right" {
                    continue;
                }

                // Transform these detections to overview space
                let crop_size = crop.image.size()?;
                let ez_w = crop_size.width as f32;
                let ez_h = crop_size.height as f32;

                for det in &results[i].detections {
                    let ov_bbox_px = transform_ez_to_overview(
                        &det.bbox,
                        &crop.source_bbox,
                        ez_w,
                        ez_h,
                        &ov_bbox,
                        ov_w,
                        ov_h,
                    );

                    let mut enriched = det.clone();
                    enriched.bbox = ov_bbox_px;
                    merged_ez_detections.push(enriched);
                }
            }

            // Append merged EZ detections to the overview CropResult
            if let Some(ov_result) = results.get_mut(ov_index) {
                let overview_count = ov_result.detections.len();
                tracing::debug!(
                    "Merge phase: Before NMS - {} overview detections + {} merged EZ detections = {} total",
                    overview_count,
                    merged_ez_detections.len(),
                    overview_count + merged_ez_detections.len()
                );
                ov_result.detections.extend(merged_ez_detections);

                // FINAL NMS to remove duplicates between overview-field and EZ-highres detections
                // Use unified NMS function with diagnostic logging
                let iou_threshold = params.slice_config.nms_iou_threshold;
                let (filtered_detections, nms_stat) =
                    nms(ov_result.detections.clone(), iou_threshold);
                merge_nms_stat = Some(nms_stat);
                ov_result.detections = filtered_detections;

                tracing::debug!(
                    "Merge phase: After NMS - {} detections remaining",
                    ov_result.detections.len()
                );
            }
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

        // Create detection summary
        let mut overview_nms = None;
        let mut left_nms = None;
        let mut right_nms = None;

        for (suffix, stat) in &nms_stats_by_crop {
            match suffix.as_str() {
                "overview" => overview_nms = Some(stat.clone()),
                "left" => left_nms = Some(stat.clone()),
                "right" => right_nms = Some(stat.clone()),
                _ => {}
            }
        }

        // Count detections in each region
        let mut left_kept = 0;
        let mut right_kept = 0;
        let mut field_kept = 0;

        for result in &results {
            for det in &result.detections {
                if det.in_end_zone {
                    if result.suffix == "left" {
                        left_kept += 1;
                    } else if result.suffix == "right" {
                        right_kept += 1;
                    }
                } else if det.in_field {
                    field_kept += 1;
                }
            }
        }

        let detection_summary = DetectionSummary {
            frame_id: frame.id,
            overview_nms,
            left_nms,
            right_nms,
            merge_nms: merge_nms_stat,
            left_kept,
            right_kept,
            field_kept,
        };

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
                detection_summary: Some(detection_summary),
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

/// Compute a safe chunk size for CoreML batch inference.
/// CoreML's BNNSFilterApplyTwoInputBatch has a dtype bug that triggers
/// when processing full batches of 8. Using at most 7 per chunk ensures
/// the batch is always "partial" and avoids the buggy code path.
fn safe_chunk_size(n_tiles: usize) -> usize {
    if n_tiles <= 7 {
        // Few enough tiles that we can process them all at once
        // without ever filling a batch of 8
        n_tiles
    } else {
        // More than 7 tiles: use chunks of 7 to stay safe
        7
    }
}

/// Transform a bounding box from end-zone crop pixel coordinates to overview crop pixel coordinates.
///
/// The transform goes: EZ pixel → global normalized → overview pixel.
///
/// - `det`: bounding box in EZ crop pixel coordinates
/// - `ez_bbox`: the EZ crop's bounding box in normalized global coordinates
/// - `ez_w`, `ez_h`: EZ crop image dimensions in pixels
/// - `ov_bbox`: the overview crop's bounding box in normalized global coordinates
/// - `ov_w`, `ov_h`: overview crop image dimensions in pixels
fn transform_ez_to_overview(
    det: &BBox,
    ez_bbox: &BBox,
    ez_w: f32,
    ez_h: f32,
    ov_bbox: &BBox,
    ov_w: f32,
    ov_h: f32,
) -> BBox {
    tracing::trace!(
        "Transform: EZ det [{:.1},{:.1},{:.1},{:.1}] → EZ bbox [{:.3},{:.3},{:.3},{:.3}] ({:.0}x{:.0}) → OV bbox [{:.3},{:.3},{:.3},{:.3}] ({:.0}x{:.0})",
        det.x, det.y, det.w, det.h,
        ez_bbox.x, ez_bbox.y, ez_bbox.w, ez_bbox.h, ez_w, ez_h,
        ov_bbox.x, ov_bbox.y, ov_bbox.w, ov_bbox.h, ov_w, ov_h
    );

    // EZ pixel → global normalized
    let global_x = ez_bbox.x + (det.x / ez_w) * ez_bbox.w;
    let global_y = ez_bbox.y + (det.y / ez_h) * ez_bbox.h;
    let global_w = (det.w / ez_w) * ez_bbox.w;
    let global_h = (det.h / ez_h) * ez_bbox.h;

    tracing::trace!(
        "Transform: Global normalized: [{:.3},{:.3},{:.3},{:.3}]",
        global_x,
        global_y,
        global_w,
        global_h
    );

    // Global normalized → overview pixel
    let ov_x = ((global_x - ov_bbox.x) / ov_bbox.w) * ov_w;
    let ov_y = ((global_y - ov_bbox.y) / ov_bbox.h) * ov_h;
    let ov_det_w = (global_w / ov_bbox.w) * ov_w;
    let ov_det_h = (global_h / ov_bbox.h) * ov_h;

    tracing::trace!(
        "Transform: Overview pixel: [{:.1},{:.1},{:.1},{:.1}]",
        ov_x,
        ov_y,
        ov_det_w,
        ov_det_h
    );

    // Warn if the transformed bbox is outside the overview bounds
    if ov_x < -10.0
        || ov_y < -10.0
        || ov_x + ov_det_w > ov_w + 10.0
        || ov_y + ov_det_h > ov_h + 10.0
    {
        tracing::warn!(
            "Transform: Resulting bbox [{:.1},{:.1},{:.1},{:.1}] is outside overview bounds [0,0,{:.0},{:.0}]",
            ov_x, ov_y, ov_det_w, ov_det_h, ov_w, ov_h
        );
    }

    BBox {
        x: ov_x,
        y: ov_y,
        w: ov_det_w,
        h: ov_det_h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_ez_to_overview_identity() {
        // When EZ crop bbox == overview crop bbox, transform is identity
        let det = BBox {
            x: 100.0,
            y: 50.0,
            w: 40.0,
            h: 60.0,
        };
        let bbox = BBox {
            x: 0.1,
            y: 0.1,
            w: 0.8,
            h: 0.8,
        };
        let result = transform_ez_to_overview(&det, &bbox, 640.0, 480.0, &bbox, 640.0, 480.0);
        assert!((result.x - det.x).abs() < 0.01);
        assert!((result.y - det.y).abs() < 0.01);
        assert!((result.w - det.w).abs() < 0.01);
        assert!((result.h - det.h).abs() < 0.01);
    }

    #[test]
    fn test_transform_ez_to_overview_left_crop() {
        // Left EZ crop covers the left third of the overview
        // Overview: x=0.0, y=0.0, w=1.0, h=0.5 → 1920x960 pixels
        // Left EZ: x=0.0, y=0.0, w=0.33, h=0.5 → 640x960 pixels
        let ov_bbox = BBox {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 0.5,
        };
        let ez_bbox = BBox {
            x: 0.0,
            y: 0.0,
            w: 0.33,
            h: 0.5,
        };

        // Detection at center of EZ crop: (320, 480) in EZ pixels
        let det = BBox {
            x: 300.0,
            y: 460.0,
            w: 40.0,
            h: 40.0,
        };

        let result =
            transform_ez_to_overview(&det, &ez_bbox, 640.0, 960.0, &ov_bbox, 1920.0, 960.0);

        // (300/640)*0.33 = 0.1546875 global x → (0.1546875/1.0)*1920 = 297.0 overview px
        assert!((result.x - 297.0).abs() < 1.0, "x: {}", result.x);
        // y: (460/960)*0.5 = 0.2395833 global → (0.2395833/0.5)*960 = 460 overview px
        assert!((result.y - 460.0).abs() < 1.0, "y: {}", result.y);
        // w: (40/640)*0.33 = 0.020625 global → (0.020625/1.0)*1920 = 39.6 overview px
        assert!((result.w - 39.6).abs() < 1.0, "w: {}", result.w);
    }
}
