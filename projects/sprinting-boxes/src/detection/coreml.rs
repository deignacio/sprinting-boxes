//! CoreML-based object detector for macOS (zero-copy GPU pipeline).
#![cfg(target_os = "macos")]

use anyhow::Result;
use objc2::rc::autoreleasepool;
use objc2_foundation::{NSString, NSURL};
use objc2_core_ml::{MLModel, MLModelConfiguration, MLComputeUnits};
use opencv::core::Mat;
use opencv::prelude::MatTraitConst;

use crate::detection::{Detection, Detector};
use crate::detection::ffi;

pub struct CoremlDetector {
    model: objc2::rc::Retained<MLModel>,
}

impl CoremlDetector {
    /// Load a .mlmodel or .mlpackage from `model_path`.
    pub fn new(model_path: &str) -> Result<Self> {
        use std::path::Path;

        let model_path = Path::new(model_path);
        if !model_path.exists() {
            anyhow::bail!("Model not found at {}", model_path.display());
        }

        let path_str = model_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid path encoding"))?;

        // Wrap model loading in autoreleasepool to drain intermediate objects created
        // during model compilation and loading. This prevents accumulation in the
        // implicit root pool that could cause segfaults during shutdown.
        let detector = autoreleasepool(|_| {
            let ns_path = NSString::from_str(path_str);
            let model_url = NSURL::fileURLWithPath(&ns_path);

            let compiled_url = unsafe {
                #[allow(deprecated)]
                MLModel::compileModelAtURL_error(model_url.as_ref())
                    .map_err(|err| anyhow::anyhow!("Failed to compile CoreML model: {:?}", err))?
            };

            let config = unsafe {
                let cfg = MLModelConfiguration::new();
                cfg.setComputeUnits(MLComputeUnits::All);
                cfg
            };

            let model = unsafe {
                MLModel::modelWithContentsOfURL_configuration_error(
                    compiled_url.as_ref(),
                    &config,
                )
                .map_err(|err| anyhow::anyhow!("Failed to load compiled CoreML model: {:?}", err))?
            };

            Ok::<_, anyhow::Error>(CoremlDetector { model })
            // Pool drains here, releasing ns_path, model_url, compiled_url, config
            // The model itself survives as a Retained<>
        })?;

        Ok(detector)
    }
}

impl Detector for CoremlDetector {
    fn detect(&self, tile: &Mat) -> Result<Vec<Detection>> {
        let pb = mat_to_pixel_buffer(tile)?;

        let provider = unsafe { ffi::create_feature_provider_from_pixel_buffer(&pb)? };

        // Extract both outputs in a single FFI call without holding ObjC object references
        // across the boundary. This keeps only Rust-owned Vec<f32> data in Rust land.
        let (confidence_data, coordinates_data) = unsafe { ffi::run_prediction_and_extract(&self.model, &*provider)? };

        // Drop ObjC objects immediately
        drop(provider);
        drop(pb);

        if confidence_data.is_empty() || coordinates_data.is_empty() {
            return Ok(Vec::new());
        }

        // D-FINE output layout:
        //   confidence: (N, 80) — per-class scores
        //   coordinates: (N, 4) — [cx, cy, w, h] normalized [0,1]
        let num_detections = confidence_data.len() / 80;
        let mut detections = Vec::new();

        for det_idx in 0..num_detections {
            let class_start = det_idx * 80;
            let class_probs = &confidence_data[class_start..class_start + 80];

            let (best_class_id, best_confidence) = class_probs
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(id, &conf)| (id, conf))
                .unwrap_or((0, 0.0));

            if best_confidence < 0.5 {
                continue;
            }

            // cxcywh → xyxy (normalized [0,1])
            let box_start = det_idx * 4;
            let cx = coordinates_data[box_start] as f32;
            let cy = coordinates_data[box_start + 1] as f32;
            let w = coordinates_data[box_start + 2] as f32;
            let h = coordinates_data[box_start + 3] as f32;

            detections.push(Detection {
                x_min: cx - w / 2.0,
                y_min: cy - h / 2.0,
                x_max: cx + w / 2.0,
                y_max: cy + h / 2.0,
                confidence: best_confidence,
                class_id: Some(best_class_id),
                class_name: None,
            });
        }

        Ok(detections)
    }
}

/// Convert an OpenCV Mat (BGR or BGRA) into a CVPixelBuffer for CoreML input.
fn mat_to_pixel_buffer(
    mat: &Mat,
) -> Result<objc2::rc::Retained<objc2_core_video::CVPixelBuffer>> {
    let width = mat.cols() as usize;
    let height = mat.rows() as usize;
    let channels = mat.channels() as usize;

    if channels != 3 && channels != 4 {
        anyhow::bail!("Mat must have 3 or 4 channels, got {}", channels);
    }

    if channels == 3 {
        let mut bgra = Mat::default();
        opencv::imgproc::cvt_color_def(mat, &mut bgra, opencv::imgproc::COLOR_BGR2BGRA)?;

        let data = bgra.data() as *const u8;
        if data.is_null() {
            anyhow::bail!("BGRA Mat data pointer is null");
        }
        let bytes_per_row = width * 4;
        let slice = unsafe { std::slice::from_raw_parts(data, height * bytes_per_row) };
        unsafe { ffi::create_pixel_buffer_from_bgra_data(slice, width, height, bytes_per_row) }
    } else {
        let data = mat.data() as *const u8;
        if data.is_null() {
            anyhow::bail!("Mat data pointer is null");
        }
        let bytes_per_row = width * 4;
        let slice = unsafe { std::slice::from_raw_parts(data, height * bytes_per_row) };
        unsafe { ffi::create_pixel_buffer_from_bgra_data(slice, width, height, bytes_per_row) }
    }
}
