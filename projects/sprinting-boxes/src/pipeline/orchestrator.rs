// Pipeline orchestrator: manages video processing lifecycle
//
// Coordinates reader and crop workers, tracks processing state,
// and provides SSE progress streaming.

pub use crate::pipeline::types::ProcessingState;
use crate::pipeline::types::RawFrame;
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
    let reader = OpencvReader::new(path_str, sample_rate).with_context(|| {
        format!(
            "Failed to open video at: '{}' (Exists: {}), CWD: '{:?}'",
            path_str,
            video_path.exists(),
            std::env::current_dir().unwrap_or_default()
        )
    })?;

    let total_frames = reader.frame_count()?;

    // Create processing state
    let state = Arc::new(ProcessingState::new(
        run_context.run_id.clone(),
        total_frames,
    ));
    register_processing(&run_context.run_id, state.clone());

    // Create channel
    let (tx, rx) = channel::bounded::<RawFrame>(32);

    // Clone for workers
    let state_reader = state.clone();
    let state_crop = state.clone();
    let output_dir = run_context.output_dir.clone();

    // Spawn reader thread
    thread::spawn(move || {
        let reader: Box<dyn VideoReader> = Box::new(reader);
        if let Err(e) = crate::pipeline::reader::read_worker(reader, tx, state_reader) {
            tracing::error!("Reader worker failed: {}", e);
        }
    });

    // Spawn crop worker thread
    thread::spawn(move || {
        crate::pipeline::crop::save_crops_worker(rx, configs, output_dir, state_crop);
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
