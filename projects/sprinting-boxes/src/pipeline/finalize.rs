use crate::pipeline::types::{DetectedFrame, ProcessingState};
use anyhow::Result;
use crossbeam::channel::Receiver;
use opencv::core::Mat;
use opencv::core::{Point, Scalar, Vector};
use opencv::imgproc::{polylines, rectangle, LINE_8};
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

        let color = if d.is_counted {
            Scalar::new(0.0, 255.0, 0.0, 0.0) // Green
        } else {
            Scalar::new(0.0, 0.0, 255.0, 0.0) // Red
        };

        rectangle(&mut draw_img, rect, color, 2, LINE_8, 0)?;
    }

    Ok(draw_img)
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
        fs::create_dir_all(&crops_dir)?;
    }

    let mut all_results = Vec::new();
    tracing::info!(
        "Finalize worker started. output_dir: {:?}, save_crops: {}",
        output_dir,
        save_crops
    );

    for frame in rx {
        let start_inst = Instant::now();

        if !state.is_active.load(Ordering::Relaxed) {
            break;
        }

        for result in &frame.results {
            if !save_crops {
                continue;
            }

            if let Some(img) = &result.image {
                // Save raw crop (no annotations)
                let filename = format!("frame_{:06}_{}.jpg", frame.id, result.suffix);
                let path = crops_dir.join(&filename);
                if let Err(e) =
                    opencv::imgcodecs::imwrite(path.to_str().unwrap(), img, &Vector::new())
                {
                    tracing::warn!("Failed to write crop image {}: {}", filename, e);
                }
            }
        }

        all_results.push(frame.clone());

        let duration_ms = start_inst.elapsed().as_secs_f64() * 1000.0;
        state.update_stage("finalize", 1, duration_ms);

        // Periodically save detections.json (every 25 frames) so dashboard works mid-run
        if !all_results.is_empty() && all_results.len() % 25 == 0 {
            let results_path = output_dir.join("detections.json");
            match serde_json::to_string(&all_results) {
                Ok(json) => {
                    let _ = fs::write(results_path, json);
                }
                Err(e) => tracing::warn!("Failed to serialize incremental detections: {}", e),
            }
        }
    }

    tracing::info!(
        "Finalize worker finished processing {} frames. Saving detections.json...",
        all_results.len()
    );

    // Save final detections.json
    let results_path = output_dir.join("detections.json");
    let json = serde_json::to_string_pretty(&all_results)?;
    fs::write(&results_path, json)?;
    tracing::info!("Saved final detections.json to {:?}", results_path);

    state.is_complete.store(true, Ordering::Relaxed);
    state.is_active.store(false, Ordering::Relaxed);

    Ok(())
}
