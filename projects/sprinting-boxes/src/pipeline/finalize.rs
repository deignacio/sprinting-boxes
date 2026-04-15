use crate::pipeline::types::{
    polygon_to_compact, CompactCropData, CompactDetection, CompactDetectionFile, CompactFrameData,
    CompactRegion, DetectedFrame, ProcessingState,
};
use anyhow::Result;
use crossbeam::channel::Receiver;
use csv::Writer;
use opencv::core::Mat;
use opencv::core::{Point, Scalar, Vector};
use opencv::imgproc::{circle, polylines, rectangle, LINE_8};
use opencv::prelude::MatTraitConst;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

/// Draw annotations (polygons and bounding boxes) onto a crop image.
/// This function is used for on-demand rendering in the web handler.
pub fn draw_annotations(
    crop_img: &Mat,
    result: &crate::pipeline::types::CropResult,
    frame: Option<&DetectedFrame>,
) -> Result<Mat> {
    let mut draw_img = crop_img.clone();

    // 1. Draw regional polygons if they exist
    if !result.regions.is_empty() {
        for region in result.regions.iter() {
            let color = match region.name.as_str() {
                "left" => Scalar::new(255.0, 100.0, 100.0, 0.0), // Light Blue/Cyan-ish
                "right" => Scalar::new(100.0, 255.0, 100.0, 0.0), // Green
                "field" => Scalar::new(200.0, 200.0, 200.0, 0.0), // Gray
                _ => Scalar::new(255.0, 255.0, 255.0, 0.0),      // White
            };

            let pts: Vec<Point> = region
                .polygon
                .iter()
                .map(|p| Point::new(p.x as i32, p.y as i32))
                .collect();

            if !pts.is_empty() {
                let mut pts_vec = Vector::<Point>::new();
                for p in pts {
                    pts_vec.push(p);
                }
                let mut contours = Vector::<Vector<Point>>::new();
                contours.push(pts_vec);
                polylines(&mut draw_img, &contours, true, color, 2, LINE_8, 0)?;
            }
        }
    } else {
        // Fallback to legacy single polygons
        // 1. Draw Original Polygon (Light Blue)
        let pts_orig: Vec<Point> = result
            .original_polygon
            .iter()
            .map(|p| Point::new(p.x as i32, p.y as i32))
            .collect();
        if !pts_orig.is_empty() {
            let mut pts_vec = Vector::<Point>::new();
            for p in pts_orig {
                pts_vec.push(p);
            }
            let mut contours = Vector::<Vector<Point>>::new();
            contours.push(pts_vec);
            let color = Scalar::new(255.0, 200.0, 100.0, 0.0); // Light Blue
            polylines(&mut draw_img, &contours, true, color, 2, LINE_8, 0)?;
        }

        // 2. Draw Effective Polygon (Orange)
        let pts_eff: Vec<Point> = result
            .effective_polygon
            .iter()
            .map(|p| Point::new(p.x as i32, p.y as i32))
            .collect();
        if !pts_eff.is_empty() {
            let mut pts_vec = Vector::<Point>::new();
            for p in pts_eff {
                pts_vec.push(p);
            }
            let mut contours = Vector::<Vector<Point>>::new();
            contours.push(pts_vec);
            let color = Scalar::new(0.0, 165.0, 255.0, 0.0); // Orange
            polylines(&mut draw_img, &contours, true, color, 2, LINE_8, 0)?;
        }
    }

    // 2. Draw Detections
    for d in &result.detections {
        let rect = opencv::core::Rect::new(
            d.bbox.x as i32,
            d.bbox.y as i32,
            d.bbox.w as i32,
            d.bbox.h as i32,
        );

        let color = if d.in_end_zone {
            Scalar::new(0.0, 255.0, 0.0, 0.0) // Green (End Zone)
        } else if d.in_field {
            Scalar::new(255.0, 0.0, 0.0, 0.0) // Blue (Field)
        } else {
            Scalar::new(0.0, 0.0, 255.0, 0.0) // Red (Neither)
        };

        rectangle(&mut draw_img, rect, color, 2, LINE_8, 0)?;
    }

    // 3. Draw CoM and StdDev (if frame provided and crop is overview)
    if let Some(f) = frame {
        if result.suffix == "overview" {
            if let (Some(cx_norm), Some(cy_norm), Some(std_dev_norm)) =
                (f.com_x, f.com_y, f.std_dev)
            {
                let w = draw_img.cols() as f32;
                let h = draw_img.rows() as f32;
                let cx = (cx_norm * w) as i32;
                let cy = (cy_norm * h) as i32;

                // StdDev Visualization
                // Normalized std_dev means 1.0 = (diagonal / 3.0)
                let diagonal = (w * w + h * h).sqrt();
                let normalization_factor = diagonal / 3.0;
                let raw_std_dev = std_dev_norm * normalization_factor;

                // Requested scaling: "1/8 of the current length"
                let radius = (raw_std_dev / 8.0) as i32;

                if radius > 0 {
                    // Outlined circle for StdDev
                    circle(
                        &mut draw_img,
                        Point { x: cx, y: cy },
                        radius,
                        Scalar::new(0.0, 165.0, 255.0, 0.0), // Orange
                        2,
                        LINE_8,
                        0,
                    )?;
                }

                // CoM Dot (Filled)
                circle(
                    &mut draw_img,
                    Point { x: cx, y: cy },
                    4,                                   // Fixed small radius for the center point
                    Scalar::new(0.0, 165.0, 255.0, 0.0), // Orange
                    -1,                                  // Filled
                    LINE_8,
                    0,
                )?;
            }
        }
    }

    Ok(draw_img)
}

/// Convert a DetectedFrame to CompactFrameData for optimized JSON output
fn convert_to_compact(frame: &DetectedFrame) -> CompactFrameData {
    let mut crops = HashMap::new();

    for result in &frame.results {
        let compact_detections: Vec<CompactDetection> = result
            .detections
            .iter()
            .map(|d| CompactDetection {
                x: d.bbox.x,
                y: d.bbox.y,
                w: d.bbox.w,
                h: d.bbox.h,
                confidence: d.confidence,
                in_end_zone: d.in_end_zone,
                in_field: d.in_field,
            })
            .collect();

        let compact_crop_data = if result.suffix == "overview" {
            // Overview: include regions
            let compact_regions: Vec<CompactRegion> = result
                .regions
                .iter()
                .map(|r| CompactRegion {
                    name: r.name.clone(),
                    polygon: polygon_to_compact(&r.polygon),
                })
                .collect();

            CompactCropData {
                detections: compact_detections,
                regions: Some(compact_regions),
                source_bbox: None,
            }
        } else {
            // EZ crops (left/right): include source_bbox
            CompactCropData {
                detections: compact_detections,
                regions: None,
                source_bbox: Some(result.bbox),
            }
        };

        crops.insert(result.suffix.clone(), compact_crop_data);
    }

    CompactFrameData {
        id: frame.id,
        crops,
    }
}

/// Finalize worker: receives detected frames, draws detections/polygons, and saves results.
pub fn finalize_worker(
    rx: Receiver<DetectedFrame>,
    output_dir: PathBuf,
    save_crops: bool,
    state: Arc<ProcessingState>,
) -> Result<()> {
    let crops_dir = output_dir.join("crops");
    if save_crops {
        let _ = fs::create_dir_all(&crops_dir);
    }

    let mut compact_file = CompactDetectionFile::new();

    // Create CSV writer for detection summaries
    let summary_path = output_dir.join("detection_summary.csv");
    let mut summary_writer = Writer::from_path(&summary_path)
        .map_err(|e| anyhow::anyhow!("Failed to create detection_summary.csv: {}", e))?;

    // Write CSV header
    summary_writer.write_record([
        "frame_id",
        "overview_original",
        "overview_suppressed",
        "overview_close_but_kept",
        "overview_kept",
        "left_original",
        "left_suppressed",
        "left_close_but_kept",
        "left_kept",
        "right_original",
        "right_suppressed",
        "right_close_but_kept",
        "right_kept",
        "merge_original",
        "merge_suppressed",
        "merge_close_but_kept",
        "merge_kept",
        "left_region_kept",
        "right_region_kept",
        "field_region_kept",
    ])?;

    tracing::info!(
        "Finalize worker started. output_dir: {:?}, save_crops: {:?}",
        output_dir,
        save_crops,
    );

    let target_filename = "detections.json";

    for frame in rx {
        let start_inst = Instant::now();

        if !state.is_active.load(Ordering::Relaxed) {
            break;
        }

        // Write detection summary to CSV
        if let Some(summary) = &frame.detection_summary {
            let overview = summary.overview_nms.as_ref();
            let left = summary.left_nms.as_ref();
            let right = summary.right_nms.as_ref();
            let merge = summary.merge_nms.as_ref();

            summary_writer.write_record([
                &summary.frame_id.to_string(),
                &overview
                    .map(|s| s.original_count.to_string())
                    .unwrap_or_default(),
                &overview
                    .map(|s| s.suppressed_count.to_string())
                    .unwrap_or_default(),
                &overview
                    .map(|s| s.close_but_kept_count.to_string())
                    .unwrap_or_default(),
                &overview
                    .map(|s| s.kept_count.to_string())
                    .unwrap_or_default(),
                &left
                    .map(|s| s.original_count.to_string())
                    .unwrap_or_default(),
                &left
                    .map(|s| s.suppressed_count.to_string())
                    .unwrap_or_default(),
                &left
                    .map(|s| s.close_but_kept_count.to_string())
                    .unwrap_or_default(),
                &left.map(|s| s.kept_count.to_string()).unwrap_or_default(),
                &right
                    .map(|s| s.original_count.to_string())
                    .unwrap_or_default(),
                &right
                    .map(|s| s.suppressed_count.to_string())
                    .unwrap_or_default(),
                &right
                    .map(|s| s.close_but_kept_count.to_string())
                    .unwrap_or_default(),
                &right.map(|s| s.kept_count.to_string()).unwrap_or_default(),
                &merge
                    .map(|s| s.original_count.to_string())
                    .unwrap_or_default(),
                &merge
                    .map(|s| s.suppressed_count.to_string())
                    .unwrap_or_default(),
                &merge
                    .map(|s| s.close_but_kept_count.to_string())
                    .unwrap_or_default(),
                &merge.map(|s| s.kept_count.to_string()).unwrap_or_default(),
                &summary.left_kept.to_string(),
                &summary.right_kept.to_string(),
                &summary.field_kept.to_string(),
            ])?;
        }

        if save_crops {
            for result in &frame.results {
                if let Some(img) = &result.image {
                    let filename = format!("frame_{:06}_{}.jpg", frame.id, result.suffix);
                    let path = crops_dir.join(&filename);
                    if let Err(e) =
                        opencv::imgcodecs::imwrite(path.to_str().unwrap(), img, &Vector::new())
                    {
                        tracing::warn!("Failed to write crop image {}: {}", filename, e);
                    }
                }
            }
        }

        // Convert to compact format
        let compact_frame = convert_to_compact(&frame);

        compact_file.frames.push(compact_frame);

        let duration_ms = start_inst.elapsed().as_secs_f64() * 1000.0;
        state.update_stage("finalize", 1, duration_ms);

        // Periodically save
        if !compact_file.frames.is_empty() && compact_file.frames.len().is_multiple_of(25) {
            let results_path = output_dir.join(target_filename);
            let json = serde_json::to_string(&compact_file).unwrap_or_default();
            let _ = fs::write(results_path, json);
        }
    }

    tracing::info!(
        "Finalize worker finished processing {} frames. Saving {}...",
        compact_file.frames.len(),
        target_filename
    );

    // Save final detections in compact format
    let results_path = output_dir.join(target_filename);
    let json = serde_json::to_string_pretty(&compact_file)?;
    fs::write(&results_path, json)?;
    tracing::info!("Saved final {} to {:?}", target_filename, results_path);

    // Flush detection summary CSV
    summary_writer.flush()?;
    tracing::info!("Saved detection summary to {:?}", summary_path);

    state.is_complete.store(true, Ordering::Relaxed);
    state.is_active.store(false, Ordering::Relaxed);

    Ok(())
}
