use crate::detection;
use crate::detection::slicing::{
    generate_tiles, nms, HbbWrapper, SliceConfig,
};
use crate::geometry::transform_ez_to_overview;
use crate::pipeline::types::{
    BBox, CropResult, DetectedFrame, DetectionSummary, EnrichedDetection, PreprocessedFrame,
    ProcessingState,
};
use anyhow::Result;
use crossbeam::channel::{Receiver, Sender};
use opencv::prelude::MatTraitConst;
use std::sync::Arc;
use std::time::Instant;
use usls::Hbb;

/// Parameters for the detection worker to avoid too many arguments clippy warning.
pub struct DetectionParams {
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
    // Create the appropriate detector (CoreML on macOS, ONNX fallback on other platforms)
    let detector = detection::create_detector()?;

    let slicing_enabled = params.slice_config.is_enabled();
    tracing::info!(
        "Detection worker started with CoreML GPU pipeline and slice_config: {:?}",
        params.slice_config
    );

    for frame in rx {
        // Exit immediately if stop_processing was called (feature and finalize workers do the same)
        if !state.is_active.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

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
            tile: crate::detection::slicing::Tile,
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
                    tile: crate::detection::slicing::Tile {
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

        // 2. Inference Phase: Run detector on all tiles
        let mut detections_by_crop = vec![Vec::new(); frame.crops.len()];
        if !all_queued_tiles.is_empty() {
            for queued in all_queued_tiles.into_iter() {
                let detections = detector.detect(&queued.tile.image)?;

                for det in detections {
                    if det.confidence < params.min_conf {
                        continue;
                    }

                    // Detections are in normalized coordinates [0,1] relative to the model input size.
                    // D-FINE model expects 640x640 input. Map coordinates back to crop pixel space.
                    // ASSUMPTION: If the model is changed to a different input size, these
                    // calculations must be updated accordingly.
                    const MODEL_SIZE: f32 = 640.0;

                    let x1 = det.x_min * MODEL_SIZE + queued.tile.x_offset as f32;
                    let y1 = det.y_min * MODEL_SIZE + queued.tile.y_offset as f32;
                    let x2 = det.x_max * MODEL_SIZE + queued.tile.x_offset as f32;
                    let y2 = det.y_max * MODEL_SIZE + queued.tile.y_offset as f32;

                    // Create usls::Hbb with detection info for downstream processing
                    let mut hbb = Hbb::default().with_xyxy(x1, y1, x2, y2).with_confidence(det.confidence);

                    if let Some(class_id) = det.class_id {
                        hbb = hbb.with_id(class_id as usize);
                    }
                    if let Some(class_name) = &det.class_name {
                        hbb = hbb.with_name(class_name.as_str());
                    }

                    detections_by_crop[queued.crop_index].push(hbb);
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
