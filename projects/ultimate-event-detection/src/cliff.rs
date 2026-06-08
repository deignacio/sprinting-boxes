use std::collections::BTreeMap;

/// Configuration for cliff detection.
#[derive(Clone, Debug)]
pub struct CliffDetectorConfig {
    pub min_drop: f32,
    pub min_prepoint_duration: usize,
    pub min_post_duration: usize,
    pub max_post_proba: f32,
    pub absolute_threshold: f32,
    pub min_gap: usize,
    pub smoothing_window: usize,
    pub video_start_prepoint_threshold: f32,
}

impl Default for CliffDetectorConfig {
    fn default() -> Self {
        Self {
            min_drop: 0.15,
            min_prepoint_duration: 10,
            min_post_duration: 10,
            max_post_proba: 0.55,
            absolute_threshold: 0.5,
            min_gap: 20,
            smoothing_window: 3,
            video_start_prepoint_threshold: 0.5,
        }
    }
}

fn median(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

/// Check if a cliff occurs at `center_idx` in a probability slice.
/// Useful for batch/offline processing where all scores are known upfront.
///
/// A cliff is detected when:
/// 1. Pre-point plateau: median score >= 0.5 for `min_prepoint_duration` frames
/// 2. Sharp drop: effective drop >= `min_drop`
/// 3. Next frame score <= `absolute_threshold`
/// 4. Post-point stability: median score <= `max_post_proba` for `min_post_duration` frames
pub fn is_cliff_at(config: &CliffDetectorConfig, probabilities: &[f32], center_idx: usize) -> bool {
    if center_idx + config.min_post_duration >= probabilities.len() {
        return false;
    }

    let smoothed: Vec<f32> = if config.smoothing_window > 1 {
        (0..probabilities.len())
            .map(|i| {
                let start = i.saturating_sub(config.smoothing_window - 1);
                let slice = &probabilities[start..i + 1];
                slice.iter().sum::<f32>() / slice.len() as f32
            })
            .collect()
    } else {
        probabilities.to_vec()
    };

    let i = center_idx;
    if i + 1 >= smoothed.len() {
        return false;
    }

    let drop = smoothed[i] - smoothed[i + 1];
    let start_w = i.saturating_sub(config.smoothing_window - 1);
    let cumulative_drop = smoothed[start_w] - smoothed[i + 1];
    let effective_drop = drop.max(cumulative_drop);

    if effective_drop < config.min_drop {
        return false;
    }
    if smoothed[i + 1] > config.absolute_threshold {
        return false;
    }

    let start_pre = i.saturating_sub(config.min_prepoint_duration);
    let pre_window = &smoothed[start_pre..i];
    if !pre_window.is_empty() {
        let threshold = if pre_window.len() >= config.min_prepoint_duration {
            0.5
        } else {
            config.video_start_prepoint_threshold
        };
        if median(pre_window) < threshold {
            return false;
        }
    }

    let post_end = (i + 1 + config.min_post_duration).min(probabilities.len());
    let post_window = &probabilities[i + 1..post_end];
    if post_window.len() < config.min_post_duration {
        return false;
    }
    if median(post_window) > config.max_post_proba {
        return false;
    }

    true
}

/// Stateful streaming cliff detector.
///
/// Feed pre-point scores frame by frame via `push`; receive finalized cliff
/// decisions once enough post-context has been buffered. Call `flush` at the
/// end of a video to drain remaining frames.
pub struct CliffDetector {
    config: CliffDetectorConfig,
    history: BTreeMap<usize, f32>,
    last_cliff_index: Option<usize>,
    finalized_count: usize,
}

impl CliffDetector {
    pub fn new(config: CliffDetectorConfig) -> Self {
        Self {
            config,
            history: BTreeMap::new(),
            last_cliff_index: None,
            finalized_count: 0,
        }
    }

    /// Push the pre-point score for `frame_index`.
    /// Returns `(frame_index, is_cliff)` pairs for any frames now finalized.
    pub fn push(&mut self, frame_index: usize, score: f32) -> Vec<(usize, bool)> {
        self.history.insert(frame_index, score);
        self.process(false)
    }

    /// Flush all remaining buffered frames (call at end of video).
    pub fn flush(&mut self) -> Vec<(usize, bool)> {
        self.process(true)
    }

    fn process(&mut self, flush: bool) -> Vec<(usize, bool)> {
        let mut results = Vec::new();

        let keys: Vec<usize> = self.history.keys().cloned().collect();
        if keys.len() < self.config.smoothing_window {
            return results;
        }

        let post_context = self.config.min_post_duration;
        let pre_context = self.config.min_prepoint_duration + self.config.smoothing_window;
        let all_probs: Vec<f32> = keys.iter().map(|k| self.history[k]).collect();

        let end_idx = if flush {
            keys.len()
        } else if keys.len() > post_context {
            keys.len() - post_context
        } else {
            0
        };

        if end_idx <= self.finalized_count {
            return results;
        }

        for (i, &frame_idx) in keys
            .iter()
            .enumerate()
            .take(end_idx)
            .skip(self.finalized_count)
        {
            let cliff = is_cliff_at(&self.config, &all_probs, i);

            let mut finalized_cliff = false;
            if cliff {
                let gap_ok = self
                    .last_cliff_index
                    .map_or(true, |last| frame_idx - last >= self.config.min_gap);
                if gap_ok {
                    finalized_cliff = true;
                    self.last_cliff_index = Some(frame_idx);
                }
            }

            results.push((frame_idx, finalized_cliff));
        }

        self.finalized_count = end_idx;

        if self.finalized_count > pre_context + 2 {
            let keep_from_idx = self.finalized_count - pre_context - 2;
            let first_keep = keys[keep_from_idx];
            self.history.retain(|&k, _| k >= first_keep);
            self.finalized_count -= keep_from_idx;
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_cliff_detection_does_not_panic() {
        let config = CliffDetectorConfig::default();
        let mut probabilities = vec![0.8f32; 15];
        probabilities.extend_from_slice(&[0.2f32; 15]);
        let _ = is_cliff_at(&config, &probabilities, 14);
    }

    #[test]
    fn streaming_cliff_detected() {
        let config = CliffDetectorConfig {
            min_prepoint_duration: 5,
            min_post_duration: 5,
            min_gap: 10,
            smoothing_window: 1,
            ..CliffDetectorConfig::default()
        };
        let mut detector = CliffDetector::new(config);

        for i in 0..10 {
            detector.push(i, 0.9);
        }
        let mut cliff_found = false;
        for i in 10..20 {
            for (_, c) in detector.push(i, 0.1) {
                if c { cliff_found = true; }
            }
        }
        for (_, c) in detector.flush() {
            if c { cliff_found = true; }
        }
        assert!(cliff_found, "expected a cliff to be detected");
    }
}
