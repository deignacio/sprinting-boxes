/// Object detection trait and implementations.
///
/// Follows the same pattern as `video/mod.rs`: a trait with platform-specific
/// implementations selected at compile time.
///
/// - macOS: CoreML (zero-copy GPU pipeline via CVPixelBuffer)
/// - Other: ONNX/RT-DETR via the USLS library

pub mod onnx;
pub mod slicing;

#[cfg(target_os = "macos")]
pub mod ffi;
#[cfg(target_os = "macos")]
pub mod coreml;
#[cfg(target_os = "macos")]
pub mod tile_extractor;

use anyhow::Result;
use opencv::core::Mat;

/// A single object detection result.
///
/// Bounding box coordinates are normalized to [0, 1] relative to the input tile.
#[derive(Debug, Clone)]
pub struct Detection {
    pub x_min: f32,
    pub y_min: f32,
    pub x_max: f32,
    pub y_max: f32,
    pub confidence: f32,
    pub class_id: Option<usize>,
    pub class_name: Option<String>,
}

/// Object detector trait — accepts an OpenCV Mat tile, returns detected objects.
/// Note: Trait does not require Send/Sync due to Objective-C constraints on macOS.
pub trait Detector {
    fn detect(&self, tile: &Mat) -> Result<Vec<Detection>>;
}

/// Create the platform-appropriate detector with hardcoded model paths.
///
/// On macOS, uses CoreML for zero-copy GPU inference (D-FINE model).
/// On other platforms, falls back to ONNX via USLS (RT-DETR model).
pub fn create_detector() -> Result<Box<dyn Detector>> {
    #[cfg(target_os = "macos")]
    {
        const MODEL_PATH: &str = "models/dfine_n_coco.mlpackage";
        Ok(Box::new(coreml::CoremlDetector::new(MODEL_PATH)?))
    }

    #[cfg(not(target_os = "macos"))]
    {
        const MODEL_PATH: &str = "rtdetr/v2-m.onnx";
        Ok(Box::new(onnx::OnnxDetector::new(MODEL_PATH)?))
    }
}
