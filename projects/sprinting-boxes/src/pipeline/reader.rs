// Reader worker: extracts frames from video and sends them through a channel

use crate::pipeline::types::RawFrame;
use crate::video::VideoReader;
use anyhow::Result;
use crossbeam::channel::Sender;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Reads frames from a video source and sends them to the next pipeline stage.
/// Returns the total number of frames read.
use crate::pipeline::orchestrator::ProcessingState;

/// Reads frames from a video source and sends them to the next pipeline stage.
pub fn read_worker(
    mut reader: Box<dyn VideoReader>,
    tx: Sender<RawFrame>,
    state: Arc<ProcessingState>,
) -> Result<()> {
    let mut count = 0;

    loop {
        let start_inst = std::time::Instant::now();
        match reader.next_frame() {
            Ok(mat) => {
                if !state.is_active.load(Ordering::Relaxed) {
                    break;
                }

                if tx.send(RawFrame { id: count, mat }).is_err() {
                    break; // Receiver closed
                }

                count += 1;
                let duration_ms = start_inst.elapsed().as_secs_f64() * 1000.0;
                state.update_stage("reader", count, duration_ms);
            }
            Err(_) => break,
        }
    }

    Ok(())
}
