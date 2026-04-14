use crate::pipeline::types::{CropConfig, PreprocessedFrame};
use crate::video::VideoReader;
use anyhow::{anyhow, Result};
use crossbeam::channel::Sender;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::pipeline::orchestrator::ProcessingState;
use crate::pipeline::types::ReaderControl;

/// Reads existing frame crops from disk using ImageDiskReader and sends them as
/// PreprocessedFrames directly to the Detection stage (bypassing CropWorker).
pub fn image_worker(
    tx_c: Sender<PreprocessedFrame>,
    state: Arc<ProcessingState>,
    control: Arc<ReaderControl>,
    configs: Arc<Vec<CropConfig>>,
) -> Result<()> {
    use crate::video::image_reader::ImageDiskReader;
    use crate::video::image_processor;

    let mut reader: Box<dyn VideoReader> = Box::new(ImageDiskReader::new(
        &control.video_path,
        control.sample_rate,
    )?);

    // Find the overview config to attach properly
    let overview_config = configs
        .iter()
        .find(|c| c.suffix == "overview")
        .ok_or_else(|| anyhow!("Missing overview crop config for field detection"))?;

    loop {
        if !state.is_active.load(Ordering::Relaxed) {
            break;
        }

        let active = state.active_reader_workers.load(Ordering::Relaxed);
        let target = control.target_count.load(Ordering::Relaxed);
        if active > target {
            break;
        }

        let range = {
            let mut pool = control
                .range_pool
                .lock()
                .map_err(|_| anyhow!("Mutex poisoned"))?;
            pool.pop_front()
        };

        let range = match range {
            Some(r) => r,
            None => break,
        };

        for unit_id in range {
            if !state.is_active.load(Ordering::Relaxed) {
                return Ok(());
            }

            let start_inst = std::time::Instant::now();
            match reader.read_unit(unit_id) {
                Ok(mat) => {
                    let frame = image_processor::process_image_to_frame(unit_id, mat, overview_config)?;
                    if tx_c.send(frame).is_err() {
                        return Ok(()); // Receiver closed
                    }

                    let duration_ms = start_inst.elapsed().as_secs_f64() * 1000.0;
                    state.update_stage("reader", 1, duration_ms);
                    // Also pretend we "cropped" it to keep the progress bars moving
                    state.update_stage("crop", 1, 0.0);
                }
                Err(e) => {
                    tracing::error!("Image worker: failed to read unit {}: {}", unit_id, e);

                    let empty_frame = image_processor::process_empty_frame(unit_id, overview_config)?;
                    if tx_c.send(empty_frame).is_err() {
                        return Ok(());
                    }
                    state.update_stage("reader", 1, 0.0);
                    state.update_stage("crop", 1, 0.0);
                }
            }
        }
    }

    Ok(())
}
