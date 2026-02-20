use crate::pipeline::types::{DetectedFrame, ProcessingState};
use anyhow::Result;
use crossbeam::channel::Receiver;
use opencv::core::Mat;
use opencv::core::{Point, Scalar, Vector};
use opencv::imgproc::{circle, polylines, rectangle, LINE_8};
use opencv::prelude::MatTraitConst;
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

/// Finalize worker: receives detected frames, draws detections/polygons, and saves results.
pub fn finalize_worker(
    rx: Receiver<DetectedFrame>,
    output_dir: PathBuf,
    save_crops: bool,
    state: Arc<ProcessingState>,
    mode: crate::pipeline::types::PipelineMode,
) -> Result<()> {
    let crops_dir = output_dir.join("crops");
    if save_crops && mode == crate::pipeline::types::PipelineMode::Pull {
        // Only save raw image crops during the initial Pull pass
        let _ = fs::create_dir_all(&crops_dir);
    }

    let mut all_results = Vec::new();

    // In Field mode, try to load the existing pull_detections to merge into
    if mode == crate::pipeline::types::PipelineMode::Field {
        let pull_path = output_dir.join("pull_detections.json");
        if pull_path.exists() {
            if let Ok(json) = fs::read_to_string(&pull_path) {
                if let Ok(pull_data) = serde_json::from_str::<Vec<DetectedFrame>>(&json) {
                    tracing::info!(
                        "Loaded {} existing frames from pull_detections.json to merge",
                        pull_data.len()
                    );
                    // Pre-allocate assuming we process the same frame sequence
                    all_results = pull_data;
                }
            }
        } else {
            tracing::warn!(
                "Field mode started but pull_detections.json not found at {:?}",
                pull_path
            );
        }
    }

    tracing::info!(
        "Finalize worker started. output_dir: {:?}, save_crops: {:?}, mode: {:?}",
        output_dir,
        save_crops,
        mode
    );

    let target_filename = match mode {
        crate::pipeline::types::PipelineMode::Pull => "pull_detections.json",
        crate::pipeline::types::PipelineMode::Field => "detections.json",
    };

    for frame in rx {
        let start_inst = Instant::now();

        if !state.is_active.load(Ordering::Relaxed) {
            break;
        }

        // We only save raw crop images in Pull mode
        if save_crops && mode == crate::pipeline::types::PipelineMode::Pull {
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

        if mode == crate::pipeline::types::PipelineMode::Field {
            // MERGE LOGIC: We already loaded `pull_detections.json` into `all_results`.
            // The incoming `frame` from FeatureWorker is already merged (metrics + detections).
            if let Some(idx) = all_results.iter().position(|f| f.id == frame.id) {
                all_results[idx] = frame.clone();
            } else {
                all_results.push(frame.clone());
            }
        } else {
            // Pull mode: just accumulate
            all_results.push(frame.clone());
        }

        let duration_ms = start_inst.elapsed().as_secs_f64() * 1000.0;
        state.update_stage("finalize", 1, duration_ms);

        // Periodically save
        if !all_results.is_empty() && all_results.len() % 25 == 0 {
            let results_path = output_dir.join(target_filename);
            let json = serde_json::to_string(&all_results).unwrap_or_default();
            let _ = fs::write(results_path, json);
        }
    }

    tracing::info!(
        "Finalize worker finished processing {} frames. Saving {}...",
        all_results.len(),
        target_filename
    );

    // Save final detections
    let results_path = output_dir.join(target_filename);
    let json = serde_json::to_string_pretty(&all_results)?;
    fs::write(&results_path, json)?;
    tracing::info!("Saved final {} to {:?}", target_filename, results_path);

    state.is_complete.store(true, Ordering::Relaxed);
    state.is_active.store(false, Ordering::Relaxed);

    Ok(())
}
