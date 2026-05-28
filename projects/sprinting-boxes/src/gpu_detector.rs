//! GPU-accelerated cliff detection using Metal on Apple Silicon.

use crate::scoring::CliffDetectorConfig;
use std::ffi::c_void;

#[cfg(not(target_os = "macos"))]
use crate::scoring::CliffDetector;

/// Detector configuration for Metal compute shader
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MetalDetectorParams {
    pub min_drop: f32,
    pub min_prepoint_duration: f32,
    pub min_post_duration: f32,
    pub max_post_proba: f32,
    pub absolute_threshold: f32,
    pub min_gap: f32,
    pub smoothing_window: f32,
}

impl From<&CliffDetectorConfig> for MetalDetectorParams {
    fn from(cfg: &CliffDetectorConfig) -> Self {
        Self {
            min_drop: cfg.min_drop,
            min_prepoint_duration: cfg.min_prepoint_duration as f32,
            min_post_duration: cfg.min_post_duration as f32,
            max_post_proba: cfg.max_post_proba,
            absolute_threshold: cfg.absolute_threshold,
            min_gap: cfg.min_gap as f32,
            smoothing_window: cfg.smoothing_window as f32,
        }
    }
}

#[cfg(target_os = "macos")]
extern "C" {
    fn gpu_detect_cliffs(
        device: *const c_void,
        command_queue: *const c_void,
        pipeline: *const c_void,
        scores: *const f32,
        score_len: u32,
        params: *const MetalDetectorParams,
        output: *mut u32,
    ) -> i32;

    fn gpu_init() -> *mut c_void;
    fn gpu_get_command_queue(device: *const c_void) -> *mut c_void;
    fn gpu_get_pipeline(device: *const c_void, lib_data: *const u8, lib_len: usize) -> *mut c_void;
    fn gpu_release_device(device: *mut c_void);
    fn gpu_release_command_queue(queue: *mut c_void);
    fn gpu_release_pipeline(pipeline: *mut c_void);
}

/// GPU-accelerated cliff detector using Metal
#[cfg(target_os = "macos")]
pub struct GPUCliffDetector {
    device: *mut c_void,
    command_queue: *mut c_void,
    pipeline: *mut c_void,
}

#[cfg(target_os = "macos")]
unsafe impl Send for GPUCliffDetector {}

#[cfg(target_os = "macos")]
unsafe impl Sync for GPUCliffDetector {}

#[cfg(target_os = "macos")]
impl GPUCliffDetector {
    /// Create a new GPU cliff detector
    pub fn new() -> Result<Self, String> {
        unsafe {
            // Initialize Metal device
            let device = gpu_init();
            if device.is_null() {
                return Err("Failed to initialize Metal device".to_string());
            }

            // Create command queue
            let command_queue = gpu_get_command_queue(device);
            if command_queue.is_null() {
                gpu_release_device(device);
                return Err("Failed to create command queue".to_string());
            }

            // Load and compile shader (embedded at build time)
            let metallib_data = include_bytes!(concat!(
                env!("OUT_DIR"),
                "/metal_detect.metallib"
            ));

            let pipeline = gpu_get_pipeline(device, metallib_data.as_ptr(), metallib_data.len());
            if pipeline.is_null() {
                gpu_release_command_queue(command_queue);
                gpu_release_device(device);
                return Err("Failed to create compute pipeline".to_string());
            }

            Ok(Self {
                device,
                command_queue,
                pipeline,
            })
        }
    }

    /// Detect cliffs in scores using GPU
    pub fn detect_cliffs_gpu(
        &self,
        scores: &[f32],
        config: &CliffDetectorConfig,
    ) -> Result<Vec<bool>, String> {
        if scores.is_empty() {
            return Ok(Vec::new());
        }

        let len = scores.len() as u32;
        let mut output = vec![0u32; scores.len()];
        let params = MetalDetectorParams::from(config);

        unsafe {
            let result = gpu_detect_cliffs(
                self.device,
                self.command_queue,
                self.pipeline,
                scores.as_ptr(),
                len,
                &params,
                output.as_mut_ptr(),
            );

            if result != 0 {
                return Err("GPU cliff detection failed".to_string());
            }
        }

        Ok(output.iter().map(|&v| v != 0).collect())
    }
}

#[cfg(target_os = "macos")]
impl Drop for GPUCliffDetector {
    fn drop(&mut self) {
        unsafe {
            gpu_release_pipeline(self.pipeline);
            gpu_release_command_queue(self.command_queue);
            gpu_release_device(self.device);
        }
    }
}

/// CPU fallback detector for non-macOS platforms
#[cfg(not(target_os = "macos"))]
pub struct GPUCliffDetector {
    _phantom: std::marker::PhantomData<()>,
}

#[cfg(not(target_os = "macos"))]
impl GPUCliffDetector {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            _phantom: std::marker::PhantomData,
        })
    }

    pub fn detect_cliffs_gpu(
        &self,
        scores: &[f32],
        config: &CliffDetectorConfig,
    ) -> Result<Vec<bool>, String> {
        let detector = CliffDetector::new(config.clone());
        Ok((0..scores.len())
            .map(|i| detector.is_cliff_at(scores, i))
            .collect())
    }
}
