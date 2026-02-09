use crate::pipeline::types::{DetectedFrame, ProcessingState};
use anyhow::Result;
use crossbeam::channel::Receiver;
use opencv::core::{Point, Scalar, Vector};
use opencv::imgproc::{polylines, rectangle, LINE_8};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

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
                let mut draw_img = img.clone();

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

                // 3. Draw Detections
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

                // 4. Save
                let filename = format!("frame_{:06}_{}.jpg", frame.id, result.suffix);
                let path = crops_dir.join(&filename);
                if let Err(e) =
                    opencv::imgcodecs::imwrite(path.to_str().unwrap(), &draw_img, &Vector::new())
                {
                    tracing::warn!("Failed to write crop image {}: {}", filename, e);
                }
            }
        }

        all_results.push(frame.clone());

        let duration_ms = start_inst.elapsed().as_secs_f64() * 1000.0;
        state.update_stage("finalize", frame.id, duration_ms);
    }

    // Save final detections.json
    let results_path = output_dir.join("detections.json");
    let json = serde_json::to_string_pretty(&all_results)?;
    fs::write(results_path, json)?;

    state.is_complete.store(true, Ordering::Relaxed);
    state.is_active.store(false, Ordering::Relaxed);

    Ok(())
}
