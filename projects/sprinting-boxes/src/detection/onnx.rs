//! ONNX-based object detector (RT-DETR via USLS).
//!
//! Fallback for non-macOS platforms, using the USLS library's RT-DETR model.

#![allow(dead_code)]

use anyhow::Result;
use image::{DynamicImage, ImageBuffer, Rgb};
use opencv::core::Mat;
use opencv::prelude::{MatTraitConst, MatTraitConstManual};
use std::sync::Mutex;
use usls::models::RTDETR;
use usls::{Config, Image};

use crate::detection::{Detection, Detector};

pub struct OnnxDetector {
    model: Mutex<RTDETR>,
}

impl OnnxDetector {
    pub fn new(model_path: &str) -> Result<Self> {
        let config = Config::default()
            .with_model_file(model_path)
            .with_class_names(&usls::NAMES_COCO_80);

        #[cfg(target_os = "macos")]
        let config = config.with_model_device(usls::Device::CoreMl);

        let config = config.commit()?;
        let model = RTDETR::new(config)?;
        Ok(Self {
            model: Mutex::new(model),
        })
    }
}

impl Detector for OnnxDetector {
    fn detect(&self, tile: &Mat) -> Result<Vec<Detection>> {
        let dynamic_image = mat_to_dynamic_image(tile)?;
        let usls_image = Image::from(dynamic_image);

        // USLS requires batch processing; we wrap single tile in a batch of 1
        let mut model = self.model.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
        let results = model.forward(&[usls_image])?;

        if results.is_empty() {
            return Ok(Vec::new());
        }

        let tile_size = tile.size()?;
        let tile_w = tile_size.width as f32;
        let tile_h = tile_size.height as f32;

        // Convert usls::Hbb results to normalized Detection objects
        let detections = results[0]
            .hbbs
            .iter()
            .map(|hbb| {
                // USLS returns coordinates in pixel space (assume same scale as input tile)
                // Normalize to [0,1] relative to tile dimensions
                let x_min = hbb.xmin() / tile_w;
                let y_min = hbb.ymin() / tile_h;
                let x_max = (hbb.xmin() + hbb.width()) / tile_w;
                let y_max = (hbb.ymin() + hbb.height()) / tile_h;

                Detection {
                    x_min,
                    y_min,
                    x_max,
                    y_max,
                    confidence: hbb.confidence().unwrap_or(0.0),
                    class_id: hbb.id().map(|id| id as usize),
                    class_name: hbb.name().map(|s| s.to_string()),
                }
            })
            .collect();

        Ok(detections)
    }
}

/// Convert OpenCV Mat (BGR) to DynamicImage (RGB)
fn mat_to_dynamic_image(mat: &Mat) -> Result<DynamicImage> {
    let mut rgb_mat = Mat::default();
    opencv::imgproc::cvt_color_def(mat, &mut rgb_mat, opencv::imgproc::COLOR_BGR2RGB)?;

    let size = rgb_mat.size()?;
    let width = size.width as u32;
    let height = size.height as u32;

    if !rgb_mat.is_continuous() {
        return Err(anyhow::anyhow!("Mat is not continuous"));
    }

    let data_bytes = rgb_mat.data_bytes()?;
    let buffer = data_bytes.to_vec();

    let img_buffer = ImageBuffer::<Rgb<u8>, _>::from_vec(width, height, buffer)
        .ok_or_else(|| anyhow::anyhow!("Failed to create ImageBuffer from Mat data"))?;

    Ok(DynamicImage::ImageRgb8(img_buffer))
}
