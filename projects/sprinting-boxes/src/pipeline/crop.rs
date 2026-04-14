use crate::pipeline::types::{
    BBox, CropConfig, CropData, PreprocessedFrame, ProcessingState, RawFrame, RegionalPolygon, FrameData,
};
use anyhow::Result;
use crossbeam::channel::{Receiver, Sender};
use opencv::prelude::*;
use opencv::{core, imgproc};
use std::sync::Arc;
use std::time::Instant;

/// Crops a Mat using a normalized bounding box.
pub fn crop_normalized(img: &core::Mat, bbox: &BBox) -> Result<core::Mat> {
    let size = img.size()?;
    let width = size.width as f32;
    let height = size.height as f32;

    let x = (bbox.x * width).round() as i32;
    let y = (bbox.y * height).round() as i32;
    let w = (bbox.w * width).round() as i32;
    let h = (bbox.h * height).round() as i32;

    let x_clamped = x.clamp(0, size.width);
    let y_clamped = y.clamp(0, size.height);
    let w_clamped = w.clamp(0, size.width - x_clamped);
    let h_clamped = h.clamp(0, size.height - y_clamped);

    if w_clamped <= 0 || h_clamped <= 0 {
        anyhow::bail!(
            "Invalid crop dimensions: {}x{} (bbox: {:?})",
            w_clamped,
            h_clamped,
            bbox
        );
    }

    let roi = core::Rect::new(x_clamped, y_clamped, w_clamped, h_clamped);
    let cropped = core::Mat::roi(img, roi)?;

    let mut out = core::Mat::default();
    cropped.copy_to(&mut out)?;

    Ok(out)
}

/// Apply CLAHE (Contrast Limited Adaptive Histogram Equalization) to enhance visibility
/// of dark objects in shadows. This helps detect people in dark uniforms.
fn enhance_crop(img: &core::Mat) -> Result<core::Mat> {
    let mut lab = core::Mat::default();
    imgproc::cvt_color(
        img,
        &mut lab,
        imgproc::COLOR_BGR2Lab,
        0,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;

    let mut channels = core::Vector::<core::Mat>::new();
    core::split(&lab, &mut channels)?;

    let mut clahe = imgproc::create_clahe(2.0, core::Size::new(8, 8))?;
    let mut l_enhanced = core::Mat::default();
    clahe.apply(&channels.get(0)?, &mut l_enhanced)?;

    channels.set(0, l_enhanced)?;

    let mut lab_enhanced = core::Mat::default();
    core::merge(&channels, &mut lab_enhanced)?;

    let mut result = core::Mat::default();
    imgproc::cvt_color(
        &lab_enhanced,
        &mut result,
        imgproc::COLOR_Lab2BGR,
        0,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;

    Ok(result)
}

/// Crop worker: receives raw frames, extracts configured regions, applies enhancements.
pub fn crop_worker(
    rx: Receiver<RawFrame>,
    tx: Sender<PreprocessedFrame>,
    configs: Arc<Vec<CropConfig>>,
    enable_clahe: bool,
    state: Arc<ProcessingState>,
    target_count: Arc<std::sync::atomic::AtomicUsize>,
) -> Result<()> {
    for frame in rx {
        // Dynamic scaling check
        let current_target = target_count.load(std::sync::atomic::Ordering::Relaxed);
        let current_active = state
            .active_crop_workers
            .load(std::sync::atomic::Ordering::Relaxed);

        if current_active > current_target {
            tracing::info!(
                "Crop worker scaling down: active ({}) > target ({})",
                current_active,
                current_target
            );
            // Exit after processing this frame? Or before?
            // If we consumed the frame, we must process it.
            // But we can check before next iteration.
        }

        let start_inst = Instant::now();
        let mut crop_data_list = Vec::with_capacity(configs.len());

        // Extract Mat from FrameData
        let mat = match &frame.data {
            FrameData::Mat(m) => m,
        };

        if !mat.empty() {
            for config in configs.iter() {
                let mut crop = crop_normalized(mat, &config.bbox)?;

                let crop_size = crop.size()?;
                let crop_w = crop_size.width as f32;
                let crop_h = crop_size.height as f32;

                if enable_clahe {
                    crop = enhance_crop(&crop)?;
                }

                let original_poly_local = crate::geometry::transform_polygon(
                    &config.original_polygon,
                    &config.bbox,
                    crop_w,
                    crop_h,
                );
                let effective_poly_local = crate::geometry::transform_polygon(
                    &config.effective_polygon,
                    &config.bbox,
                    crop_w,
                    crop_h,
                );
                // NEW: Transform sub-regions to local coords
                let regions_local = config
                    .regions
                    .iter()
                    .map(|r| RegionalPolygon {
                        name: r.name.clone(),
                        polygon: crate::geometry::transform_polygon(
                            &r.polygon,
                            &config.bbox,
                            crop_w,
                            crop_h,
                        ),
                        effective_polygon: crate::geometry::transform_polygon(
                            &r.effective_polygon,
                            &config.bbox,
                            crop_w,
                            crop_h,
                        ),
                    })
                    .collect();

                crop_data_list.push(CropData {
                    image: crop,
                    original_polygon: original_poly_local,
                    effective_polygon: effective_poly_local,
                    suffix: config.suffix.clone(),
                    regions: regions_local,
                    source_bbox: BBox {
                        x: config.bbox.x,
                        y: config.bbox.y,
                        w: config.bbox.w,
                        h: config.bbox.h,
                    },
                });
            }
        } else {
            tracing::warn!("Crop worker: passing through empty frame {}", frame.id);
        }

        let duration_ms = start_inst.elapsed().as_secs_f64() * 1000.0;
        state.update_stage("crop", 1, duration_ms);

        if tx
            .send(PreprocessedFrame {
                id: frame.id,
                crops: crop_data_list,
            })
            .is_err()
        {
            break;
        }

        // Check if we should exit after processing
        let current_target = target_count.load(std::sync::atomic::Ordering::Relaxed);
        let current_active = state
            .active_crop_workers
            .load(std::sync::atomic::Ordering::Relaxed);
        if current_active > current_target {
            tracing::info!("Crop worker exiting to scale down");
            break;
        }
    }

    Ok(())
}
