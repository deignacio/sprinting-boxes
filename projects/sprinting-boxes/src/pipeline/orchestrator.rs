// Pipeline orchestrator: manages video processing lifecycle
//
// Coordinates reader and crop workers, tracks processing state,
// and provides SSE progress streaming.

pub use crate::pipeline::types::ProcessingState;
use crate::run_context::RunContext;
use crate::video::opencv_reader::OpencvReader;
use crate::video::VideoReader;
use anyhow::{Context, Result};
use crossbeam::channel;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::thread;

// Global registry of active processing runs
lazy_static::lazy_static! {
    static ref PROCESSING_REGISTRY: RwLock<HashMap<String, Arc<ProcessingState>>> =
        RwLock::new(HashMap::new());
}

pub fn get_processing_state(run_id: &str) -> Option<Arc<ProcessingState>> {
    let state = PROCESSING_REGISTRY.read().unwrap().get(run_id).cloned();

    if state.is_none() {
        tracing::debug!("No processing state found for run_id: {}", run_id);
    }
    state
}

fn register_processing(run_id: &str, state: Arc<ProcessingState>) {
    tracing::info!("Registering processing state for run_id: {}", run_id);
    PROCESSING_REGISTRY
        .write()
        .unwrap()
        .insert(run_id.to_string(), state);
}

#[allow(dead_code)]
fn unregister_processing(run_id: &str) {
    PROCESSING_REGISTRY.write().unwrap().remove(run_id);
}

/// Start processing a run
pub fn start_processing(
    run_context: &RunContext,
    video_root: &Path,
) -> Result<Arc<ProcessingState>> {
    let video_path = run_context.resolve_video_path(video_root);

    // Check if already processing
    if let Some(state) = get_processing_state(&run_context.run_id) {
        if state.is_active.load(Ordering::Relaxed) {
            anyhow::bail!("Run {} is already being processed", run_context.run_id);
        }
    }

    // Load crop configs
    let crops = run_context.load_crop_configs()?;
    let pipeline_configs: Vec<crate::pipeline::types::CropConfig> = (&crops).into();
    let configs = Arc::new(pipeline_configs);

    if !video_path.exists() {
        return Err(anyhow::anyhow!("Video file NOT FOUND at: {:?}", video_path));
    }

    // Create reader
    let path_str = video_path.to_str().unwrap();
    let sample_rate = run_context.sample_rate;
    let reader = OpencvReader::new(path_str, sample_rate)
        .with_context(|| format!("Failed to open video at: '{}'", path_str))?;

    let total_frames = reader.frame_count()?;

    // Create processing state
    let state = Arc::new(ProcessingState::new(
        run_context.run_id.clone(),
        total_frames,
    ));
    register_processing(&run_context.run_id, state.clone());

    // Detection settings (could be moved to RunContext later)
    let model_path =
        std::env::var("DETECTION_MODEL_PATH").unwrap_or_else(|_| "rtdetr/v2-m.onnx".to_string());
    let slice_config = crate::pipeline::slicing::SliceConfig::new(640, 0.2); // Slicing enabled by default
    let min_conf = 0.5;

    // Create channels
    let (tx_v, rx_v) = channel::bounded::<crate::pipeline::types::RawFrame>(16);
    let (tx_c, rx_c) = channel::bounded::<crate::pipeline::types::PreprocessedFrame>(16);
    let (tx_d, rx_d) = channel::bounded::<crate::pipeline::types::DetectedFrame>(16);

    // Spawn 1: Reader
    let state_r = state.clone();
    thread::spawn(move || {
        let reader: Box<dyn VideoReader> = Box::new(reader);
        if let Err(e) = crate::pipeline::reader::read_worker(reader, tx_v, state_r) {
            tracing::error!("Reader failed: {}", e);
        }
    });

    // Spawn 2: Crop
    let state_c = state.clone();
    let configs_c = configs.clone();
    thread::spawn(move || {
        if let Err(e) = crate::pipeline::crop::crop_worker(rx_v, tx_c, configs_c, true, state_c) {
            tracing::error!("Crop worker failed: {}", e);
        }
    });

    // Spawn 3: Detect
    let state_d = state.clone();
    thread::spawn(move || {
        if let Err(e) = crate::pipeline::detection_worker::detection_worker(
            rx_c,
            tx_d,
            &model_path,
            min_conf,
            slice_config,
            state_d,
        ) {
            tracing::error!("Detection worker failed: {}", e);
        }
    });

    // Spawn 4: Finalize
    let state_f = state.clone();
    let output_dir = run_context.output_dir.clone();
    let save_visuals = std::env::var("SAVE_VISUAL_CROPS")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(true); // Default to true for now as requested

    thread::spawn(move || {
        if let Err(e) =
            crate::pipeline::finalize::finalize_worker(rx_d, output_dir, save_visuals, state_f)
        {
            tracing::error!("Finalize worker failed: {}", e);
        }
    });

    Ok(state)
}

/// Stop processing a run
pub fn stop_processing(run_id: &str) -> bool {
    if let Some(state) = get_processing_state(run_id) {
        state.is_active.store(false, Ordering::Relaxed);
        true
    } else {
        false
    }
}
