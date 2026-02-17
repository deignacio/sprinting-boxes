// Reader worker: extracts frames from video and sends them through a channel

use crate::pipeline::types::RawFrame;
use crate::video::VideoReader;
use anyhow::{anyhow, Result};
use crossbeam::channel::Sender;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Reads frames from a video source and sends them to the next pipeline stage.
/// Returns the total number of frames read.
use crate::pipeline::orchestrator::ProcessingState;

/// Reads frames from a video source in chunks from a shared pool and sends them to the pipeline.
pub fn read_worker(
    tx: Sender<RawFrame>,
    state: Arc<ProcessingState>,
    control: Arc<crate::pipeline::types::ReaderControl>,
) -> Result<()> {
    use crate::video::ffmpeg_reader::FfmpegReader;
    use crate::video::opencv_reader::OpencvReader;

    // Each worker gets its own reader instance (must be created inside the thread)
    let mut reader: Box<dyn VideoReader> = match control.backend.as_str() {
        "ffmpeg" => Box::new(FfmpegReader::new(&control.video_path, control.sample_rate)?),
        _ => Box::new(OpencvReader::new(&control.video_path, control.sample_rate)?),
    };

    loop {
        // 1. Check if we should exit (orchestrator asked us to scale down or processing stopped)
        if !state.is_active.load(Ordering::Relaxed) {
            break;
        }

        // Dynamic scaling check
        let active = state.active_reader_workers.load(Ordering::Relaxed);
        let target = control.target_count.load(Ordering::Relaxed);
        if active > target {
            break;
        }

        // 2. Get next chunk from the pool
        let range = {
            let mut pool = control
                .range_pool
                .lock()
                .map_err(|_| anyhow!("Mutex poisoned"))?;
            pool.pop_front()
        };

        let range = match range {
            Some(r) => r,
            None => break, // No more work
        };

        // 3. Read the chunk using absolute unit mapping (handled by reader)
        for unit_id in range {
            if !state.is_active.load(Ordering::Relaxed) {
                return Ok(());
            }

            let start_inst = std::time::Instant::now();
            match reader.read_unit(unit_id) {
                Ok(mat) => {
                    if tx.send(RawFrame { id: unit_id, mat }).is_err() {
                        return Ok(()); // Receiver closed
                    }
                    let duration_ms = start_inst.elapsed().as_secs_f64() * 1000.0;
                    // Increment by 1 unit completed
                    state.update_stage("reader", 1, duration_ms);
                }
                Err(_) => break, // End of stream or error in chunk
            }
        }
    }

    Ok(())
}
