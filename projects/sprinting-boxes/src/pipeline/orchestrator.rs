// Pipeline orchestrator: manages video processing lifecycle
//
// Coordinates reader and crop workers, tracks processing state,
// and provides SSE progress streaming.

pub use crate::pipeline::types::ProcessingState;
use crate::run_context::RunContext;
use crate::video::ffmpeg_reader::FfmpegReader;
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
    static ref PROCESSING_REGISTRY: RwLock<HashMap<String, Arc<PipelineManager>>> =
        RwLock::new(HashMap::new());
}

/// Control structure for detection workers
pub struct DetectionControl {
    pub source_rx: crossbeam::channel::Receiver<crate::pipeline::types::PreprocessedFrame>,
    pub result_tx:
        Arc<RwLock<Option<crossbeam::channel::Sender<crate::pipeline::types::DetectedFrame>>>>,
    pub model_path: String,
    pub min_conf: f32,
    pub slice_conf: crate::pipeline::slicing::SliceConfig,
    pub target_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub regions_to_detect: Option<Vec<String>>, // NEW: target suffixes to detect (e.g. ["left", "right"])
}

impl DetectionControl {
    pub fn get_tx(
        &self,
    ) -> Option<crossbeam::channel::Sender<crate::pipeline::types::DetectedFrame>> {
        self.result_tx.read().unwrap().clone()
    }
    pub fn close_tx(&self) {
        self.result_tx.write().unwrap().take();
    }
}

/// Control structure for crop workers
pub struct CropControl {
    pub source_rx: crossbeam::channel::Receiver<crate::pipeline::types::RawFrame>,
    pub result_tx:
        Arc<RwLock<Option<crossbeam::channel::Sender<crate::pipeline::types::PreprocessedFrame>>>>,
    pub configs: Arc<Vec<crate::pipeline::types::CropConfig>>,
    pub enable_clahe: bool,
    pub target_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl CropControl {
    pub fn get_tx(
        &self,
    ) -> Option<crossbeam::channel::Sender<crate::pipeline::types::PreprocessedFrame>> {
        self.result_tx.read().unwrap().clone()
    }
    pub fn close_tx(&self) {
        self.result_tx.write().unwrap().take();
    }
}

/// Manager that holds state and control handles for a run
pub struct PipelineManager {
    pub state: Arc<ProcessingState>,
    pub reader_control: Arc<crate::pipeline::types::ReaderControl>,
    pub detect_control: Arc<DetectionControl>,
    pub crop_control: Arc<CropControl>,
}

pub fn get_pipeline_manager(run_id: &str) -> Option<Arc<PipelineManager>> {
    PROCESSING_REGISTRY.read().unwrap().get(run_id).cloned()
}

pub fn get_processing_state(run_id: &str) -> Option<Arc<ProcessingState>> {
    get_pipeline_manager(run_id).map(|pm| pm.state.clone())
}

fn register_pipeline(run_id: &str, manager: Arc<PipelineManager>) {
    tracing::info!("Registering pipeline manager for run_id: {}", run_id);
    PROCESSING_REGISTRY
        .write()
        .unwrap()
        .insert(run_id.to_string(), manager);
}

#[allow(dead_code)]
fn unregister_pipeline(run_id: &str) {
    PROCESSING_REGISTRY.write().unwrap().remove(run_id);
}

/// Start processing a run
pub fn start_processing(
    run_context: &RunContext,
    video_root: &Path,
    model_path: &str,
    backend: &str,
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

    // Create reader based on selected backend
    let path_str = video_path.to_str().unwrap();
    let sample_rate = run_context.sample_rate;
    let reader: Box<dyn VideoReader> = match backend {
        "ffmpeg" => Box::new(
            FfmpegReader::new(path_str, sample_rate)
                .with_context(|| format!("Failed to open video with ffmpeg at: '{}'", path_str))?,
        ),
        _ => Box::new(
            OpencvReader::new(path_str, sample_rate)
                .with_context(|| format!("Failed to open video at: '{}'", path_str))?,
        ),
    };

    let total_units = reader.frame_count()?;

    // Create range pool for parallel readers (chunks of 200 sampled units)
    // Larger chunks reduce lock contention and seeking overhead.
    let chunk_size = 200;
    let mut ranges = std::collections::VecDeque::new();
    for i in (0..total_units).step_by(chunk_size) {
        let end = (i + chunk_size).min(total_units);
        ranges.push_back(i..end);
    }
    let range_pool = Arc::new(std::sync::Mutex::new(ranges));

    // Create processing state
    let state = Arc::new(ProcessingState::new(
        run_context.run_id.clone(),
        total_units,
    ));

    // Detection config (use function argument)
    let min_conf = 0.5;
    let slice_config = crate::pipeline::slicing::SliceConfig::new(640, 0.2);

    // Create channels
    // Tight bound of 2 frames per worker to prevent excessive memory usage with 8K frames
    let reader_workers_initial = 1;
    let (tx_v, rx_v) =
        channel::bounded::<crate::pipeline::types::RawFrame>(reader_workers_initial * 2);
    let (tx_c, rx_c) = channel::unbounded::<crate::pipeline::types::PreprocessedFrame>(); // Unbounded for distribution
    let (tx_d, rx_d) = channel::unbounded::<crate::pipeline::types::DetectedFrame>(); // Unbounded results

    // Target worker counts
    let target_reader = Arc::new(std::sync::atomic::AtomicUsize::new(reader_workers_initial));
    let target_crop = Arc::new(std::sync::atomic::AtomicUsize::new(1));
    let target_detect = Arc::new(std::sync::atomic::AtomicUsize::new(1));

    // Create control structures
    let reader_control = Arc::new(crate::pipeline::types::ReaderControl {
        range_pool,
        target_count: target_reader,
        tx_v: Arc::new(RwLock::new(Some(tx_v))),
        video_path: path_str.to_string(),
        backend: backend.to_string(),
        sample_rate,
    });

    let detect_control = Arc::new(DetectionControl {
        source_rx: rx_c.clone(),
        result_tx: Arc::new(RwLock::new(Some(tx_d))),
        model_path: model_path.to_string(),
        min_conf,
        slice_conf: slice_config,
        target_count: target_detect.clone(),
        regions_to_detect: None, // Default to all regions (matching existing behavior)
    });

    let crop_control = Arc::new(CropControl {
        source_rx: rx_v, // crop worker now takes from rx_v (reader output)
        result_tx: Arc::new(RwLock::new(Some(tx_c))), // outputs to rx_c (detection input)
        configs: configs.clone(),
        enable_clahe: true, // Hardcoded for now as per previous logic logic but explicit
        target_count: target_crop.clone(),
    });

    let manager = Arc::new(PipelineManager {
        state: state.clone(),
        reader_control: reader_control.clone(),
        detect_control: detect_control.clone(),
        crop_control: crop_control.clone(),
    });

    register_pipeline(&run_context.run_id, manager.clone());

    // Spawn 1: Readers
    spawn_reader_worker(state.clone(), reader_control.clone());

    // Spawn 2: Crop (Initial Worker)
    spawn_crop_worker(state.clone(), crop_control.clone());

    // Spawn 3: Detection (Initial Worker)
    spawn_detection_worker(state.clone(), detect_control.clone());

    // Spawn 4: Feature extraction
    let (tx_f, rx_f) = crossbeam::channel::unbounded();
    let state_feat = state.clone();
    let output_dir_feat = run_context.output_dir.clone();
    let team_size = run_context.team_size as usize;
    thread::spawn(move || {
        let config = crate::pipeline::feature::FeatureConfig {
            team_size: team_size,
            lookback_frames: 10,
            lookahead_frames: 15,
            output_dir: output_dir_feat,
        };
        if let Err(e) = crate::pipeline::feature::feature_worker(rx_d, tx_f, config, state_feat) {
            tracing::error!("Feature worker failed: {}", e);
        }
    });

    // Spawn 5: Finalize
    let state_f = state.clone();
    let output_dir = run_context.output_dir.clone();
    let save_visuals = std::env::var("SAVE_VISUAL_CROPS")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(true);

    thread::spawn(move || {
        if let Err(e) =
            crate::pipeline::finalize::finalize_worker(rx_f, output_dir, save_visuals, state_f)
        {
            tracing::error!("Finalize worker failed: {}", e);
        }
    });

    // Spawn 6: Supervisor (handles stage completion and channel closing)
    spawn_supervisor(manager);

    Ok(state)
}

fn spawn_reader_worker(
    state: Arc<ProcessingState>,
    control: Arc<crate::pipeline::types::ReaderControl>,
) {
    state.active_reader_workers.fetch_add(1, Ordering::Relaxed);
    let tx_v = control.get_tx().expect("Reader transmitter missing");
    std::thread::spawn(move || {
        tracing::info!("Spawning new reader worker");
        let result = crate::pipeline::reader::read_worker(tx_v, state.clone(), control);

        state.active_reader_workers.fetch_sub(1, Ordering::Relaxed);
        if let Err(e) = result {
            tracing::error!("Reader worker failed: {}", e);
        } else {
            tracing::info!("Reader worker finished gracefully");
        }
    });
}

/// Spawns a background thread that monitors the completion of each pipeline stage.
/// The supervisor ensures a sequential, clean shutdown by:
/// 1. Waiting for all Reader workers to finish sharded ranges.
/// 2. Closing the Reader -> Crop channel.
/// 3. Waiting for all Crop workers to finish.
/// 4. Closing the Crop -> Detection channel.
/// 5. Waiting for all Detection workers to finish.
/// 6. Closing the Detection -> Finalization channel.
/// 7. Updating the final total_frames count for accurate 100% progress reporting.
fn spawn_supervisor(manager: Arc<PipelineManager>) {
    let run_id = manager.state.run_id.clone();
    thread::spawn(move || {
        tracing::info!("[Supervisor:{}] Monitoring pipeline completion", run_id);

        // 1. Wait for Reader stage
        while manager.state.active_reader_workers.load(Ordering::Relaxed) > 0 {
            thread::sleep(std::time::Duration::from_millis(500));
        }
        // Check if pool is empty (double check)
        let pool_empty = manager.reader_control.range_pool.lock().unwrap().is_empty();
        if pool_empty {
            tracing::info!("[Supervisor:{}] Reader stage done. Closing tx_v.", run_id);
            // Before closing, if the reader finished early, update the total for all downstream stages
            let reader_final_count = manager
                .state
                .stages
                .read()
                .unwrap()
                .get("reader")
                .map(|s| s.current)
                .unwrap_or(0);
            if reader_final_count > 0 {
                manager.state.set_total_frames(reader_final_count);
            }
            manager.reader_control.close_tx();
        }

        // 2. Wait for Crop stage
        while manager.state.active_crop_workers.load(Ordering::Relaxed) > 0 {
            thread::sleep(std::time::Duration::from_millis(500));
        }
        tracing::info!("[Supervisor:{}] Crop stage done. Closing tx_c.", run_id);
        manager.crop_control.close_tx();

        // 3. Wait for Detection stage
        while manager.state.active_detect_workers.load(Ordering::Relaxed) > 0 {
            thread::sleep(std::time::Duration::from_millis(500));
        }
        tracing::info!(
            "[Supervisor:{}] Detection stage done. Closing tx_d.",
            run_id
        );
        manager.detect_control.close_tx();

        // 4. Wait for Finalize to finish (indicated by is_complete)
        while !manager.state.is_complete.load(Ordering::Relaxed)
            && manager.state.is_active.load(Ordering::Relaxed)
        {
            thread::sleep(std::time::Duration::from_millis(500));
        }

        tracing::info!("[Supervisor:{}] Pipeline finished. Unregistering.", run_id);
        unregister_pipeline(&run_id);
    });
}

fn spawn_detection_worker(state: Arc<ProcessingState>, control: Arc<DetectionControl>) {
    state.active_detect_workers.fetch_add(1, Ordering::Relaxed);
    let tx_d = control.get_tx().expect("Detection transmitter missing");
    std::thread::spawn(move || {
        tracing::info!("Spawning new detection worker");
        let result = crate::pipeline::detection_worker::detection_worker(
            control.source_rx.clone(),
            tx_d,
            &control.model_path,
            control.min_conf,
            control.slice_conf.clone(),
            state.clone(),
            control.target_count.clone(),
            control.regions_to_detect.clone(),
        );

        state.active_detect_workers.fetch_sub(1, Ordering::Relaxed);
        if let Err(e) = result {
            tracing::error!("Detection worker failed: {}", e);
        } else {
            tracing::info!("Detection worker finished gracefully");
        }
    });
}

fn spawn_crop_worker(state: Arc<ProcessingState>, control: Arc<CropControl>) {
    state.active_crop_workers.fetch_add(1, Ordering::Relaxed);
    let tx_c = control.get_tx().expect("Crop transmitter missing");
    std::thread::spawn(move || {
        tracing::info!("Spawning new crop worker");
        let result = crate::pipeline::crop::crop_worker(
            control.source_rx.clone(),
            tx_c,
            control.configs.clone(),
            control.enable_clahe,
            state.clone(),
            control.target_count.clone(),
        );

        state.active_crop_workers.fetch_sub(1, Ordering::Relaxed);
        if let Err(e) = result {
            tracing::error!("Crop worker failed: {}", e);
        } else {
            tracing::info!("Crop worker finished gracefully");
        }
    });
}

/// Dynamically scale the number of workers for a stage
pub fn scale_workers(run_id: &str, stage: &str, delta: i32) -> Option<usize> {
    if let Some(manager) = get_pipeline_manager(run_id) {
        let (target_atomic, stage_type) = match stage {
            "reader" => (&manager.reader_control.target_count, 0),
            "crop" => (&manager.crop_control.target_count, 1),
            "detect" => (&manager.detect_control.target_count, 2),
            _ => return None,
        };

        let current_target = target_atomic.load(Ordering::Relaxed);
        let new_target = if delta < 0 {
            current_target
                .saturating_sub(delta.unsigned_abs() as usize)
                .max(1)
        } else {
            current_target + (delta as usize)
        };

        if new_target != current_target {
            tracing::info!(
                "Scaling {} workers from {} to {}",
                stage,
                current_target,
                new_target
            );
            target_atomic.store(new_target, Ordering::Relaxed);

            // If increasing, we need to spawn new workers
            if new_target > current_target {
                let to_spawn = new_target - current_target;
                for _ in 0..to_spawn {
                    match stage_type {
                        0 => spawn_reader_worker(
                            manager.state.clone(),
                            manager.reader_control.clone(),
                        ),
                        1 => spawn_crop_worker(manager.state.clone(), manager.crop_control.clone()),
                        2 => spawn_detection_worker(
                            manager.state.clone(),
                            manager.detect_control.clone(),
                        ),
                        _ => {}
                    }
                }
            }
            // If decreasing, existing workers will check `target_count` and exit autonomously
        }
        Some(new_target)
    } else {
        None
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::slicing::SliceConfig;
    use crate::pipeline::types::ProcessingState;
    use crossbeam::channel;
    use std::sync::atomic::Ordering;

    #[test]
    fn test_scale_workers_logic() {
        // Setup mock state
        let state = Arc::new(ProcessingState::new("test_run".to_string(), 100));

        // Setup mock config
        let (tx_c, rx_c) = channel::unbounded(); // Detect input
        let (tx_d, _rx_d) = channel::unbounded(); // Detect output
        let (tx_v, rx_v) = channel::bounded(8); // Crop input (reader output)

        // Target counts
        let target_reader = Arc::new(std::sync::atomic::AtomicUsize::new(1));
        let target_crop = Arc::new(std::sync::atomic::AtomicUsize::new(1));
        let target_detect = Arc::new(std::sync::atomic::AtomicUsize::new(1));

        let reader_control = Arc::new(crate::pipeline::types::ReaderControl {
            range_pool: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
            target_count: target_reader.clone(),
            tx_v: Arc::new(RwLock::new(Some(tx_v))),
            video_path: "mock_video".to_string(),
            backend: "mock_backend".to_string(),
            sample_rate: 1.0,
        });

        let detect_control = Arc::new(DetectionControl {
            source_rx: rx_c,
            result_tx: Arc::new(RwLock::new(Some(tx_d))),
            model_path: "mock_model".to_string(),
            min_conf: 0.5,
            slice_conf: SliceConfig::new(640, 0.2),
            target_count: target_detect.clone(),
            regions_to_detect: None,
        });

        let crop_control = Arc::new(CropControl {
            source_rx: rx_v,
            result_tx: Arc::new(RwLock::new(Some(tx_c))),
            configs: Arc::new(vec![]),
            enable_clahe: true,
            target_count: target_crop.clone(),
        });

        let manager = Arc::new(PipelineManager {
            state: state.clone(),
            reader_control: reader_control.clone(),
            detect_control: detect_control.clone(),
            crop_control: crop_control.clone(),
        });

        // Register manually
        register_pipeline("test_run", manager);

        // Test scaling DETECT up: +2 -> 1 + 2 = 3
        scale_workers("test_run", "detect", 2);
        assert_eq!(target_detect.load(Ordering::Relaxed), 3);
        assert_eq!(target_reader.load(Ordering::Relaxed), 1); // Reader unchanged

        // Test scaling READER up: +1 -> 1 + 1 = 2
        scale_workers("test_run", "reader", 1);
        assert_eq!(target_reader.load(Ordering::Relaxed), 2);

        // Test scaling CROP up: +1 -> 1 + 1 = 2
        scale_workers("test_run", "crop", 1);
        assert_eq!(target_crop.load(Ordering::Relaxed), 2);

        // Test scaling DETECT down: -1 -> 3 - 1 = 2
        scale_workers("test_run", "detect", -1);
        assert_eq!(target_detect.load(Ordering::Relaxed), 2);

        // Test invalid stage
        let res = scale_workers("test_run", "invalid", 1);
        assert!(res.is_none());

        // Cleanup
        unregister_pipeline("test_run");
    }
}
