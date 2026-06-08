# ultimate-event-detection

A Rust library for detecting game events in ultimate frisbee video analysis.

Encapsulates two core algorithms extracted from [Sprinting Boxes](../../README.md).  These are my interpretations of the point-start and pull-side detection algorithms shared by Will Chen (@wpchen).

1. **Pre-point scoring** — given normalized end-zone player counts per frame, produces a `[0, 1]` score measuring how likely the frame represents a pre-point huddle state (both end zones occupied).
2. **Cliff detection** — given a time series of pre-point scores, detects the sharp drop (cliff) that signals a point starting.
3. **Pull-side detection** — given per-frame end-zone occupancy around a cliff, determines which team pulled by finding which end zone emptied first.

## Usage

```toml
[dependencies]
ultimate-event-detection = { path = "../ultimate-event-detection" }

# On macOS, opt into GPU-accelerated cliff detection
[target.'cfg(target_os = "macos")'.dependencies]
ultimate-event-detection = { path = "../ultimate-event-detection", features = ["metal"] }
```

### Pre-point score

```rust
use ultimate_event_detection::{pre_point_score, EndZoneOccupancy};

let occupancy = EndZoneOccupancy {
    left: left_count as f32 / team_size as f32,
    right: right_count as f32 / team_size as f32,
    field: field_count as f32 / team_size as f32,
};
let score = pre_point_score(&occupancy, team_size);
// score in [0, 1]: higher = more likely a pre-point huddle state
```

### Cliff detection (streaming)

Feed scores frame by frame; decisions are emitted once enough post-context is buffered.

```rust
use ultimate_event_detection::{CliffDetector, CliffDetectorConfig};

let mut detector = CliffDetector::new(CliffDetectorConfig::default());

for (frame_index, score) in scores {
    for (idx, is_cliff) in detector.push(frame_index, score) {
        if is_cliff {
            println!("cliff at frame {idx}");
        }
    }
}
// Flush remaining frames at end of video
for (idx, is_cliff) in detector.flush() {
    if is_cliff {
        println!("cliff at frame {idx}");
    }
}
```

### Cliff detection (batch)

For offline processing when all scores are already available:

```rust
use ultimate_event_detection::{is_cliff_at, CliffDetectorConfig};

let config = CliffDetectorConfig::default();
let cliffs: Vec<usize> = (0..scores.len())
    .filter(|&i| is_cliff_at(&config, &scores, i))
    .collect();
```

### Pull-side detection

```rust
use ultimate_event_detection::{detect_pull_side, EndZoneOccupancy, PullSide};

// history: (frame_index, occupancy) pairs covering lookback + lookahead around the cliff
let side = detect_pull_side(&history, 2 /* debounce_frames */);
match side {
    PullSide::Left    => println!("left team pulled"),
    PullSide::Right   => println!("right team pulled"),
    PullSide::Tie     => println!("simultaneous — tiebreaker applied"),
    PullSide::Unknown => println!("neither end zone emptied (possible false positive)"),
}
```

### GPU acceleration (macOS, `metal` feature)

```rust
use ultimate_event_detection::{GpuCliffDetector, CliffDetectorConfig};

let detector = GpuCliffDetector::new()?;
let cliff_flags = detector.detect_cliffs(&scores, &CliffDetectorConfig::default())?;
```

Without the `metal` feature, `GpuCliffDetector::detect_cliffs` falls back to the pure-Rust batch implementation automatically.

## Algorithm details

### Pre-point score

A frame scores highly when both end zones have players and the counts are roughly symmetric. Three terms are multiplied:

- **Balance** — the minimum of left/right normalized counts; requires both sides to be occupied. A single stray player on the weaker side contributes half weight to preserve signal during momentary dropouts.
- **Symmetry bonus** — penalizes imbalance between left and right counts.
- **Field term** — reserved for future use; currently always 1.0.

### Cliff detection

A cliff (point-start transition) is accepted when all four conditions hold:

1. **Pre-point plateau** — median score ≥ 0.5 over the preceding `min_prepoint_duration` frames (relaxed threshold for very early points near the start of the video).
2. **Sharp drop** — effective drop ≥ `min_drop` (the larger of the single-frame drop and the cumulative drop across the smoothing window).
3. **Absolute threshold** — the frame immediately after the cliff scores ≤ `absolute_threshold`.
4. **Post-point stability** — median score ≤ `max_post_proba` for the following `min_post_duration` frames.

A `min_gap` between successive cliffs prevents double-detection on a single transition.

### Pull-side detection

Within a window around a cliff (lookback + lookahead), the algorithm scans for the first end zone to sustain `debounce_frames` consecutive zero-count frames. The zone that reaches zero first identifies the pulling team. When both zones empty simultaneously, an earlier frame with asymmetric counts is used as a tiebreaker.

## Metal GPU shader

The `metal` feature compiles `src/metal/metal_detect.metal` — an MSL reimplementation of the cliff detection algorithm that runs each frame index as a parallel GPU thread. The Rust `is_cliff_at` function and the shader implement the same logic; if you modify the detection algorithm, update both.

## Configuration

`CliffDetectorConfig` fields and their defaults:

| Field | Default | Description |
|---|---|---|
| `min_drop` | 0.15 | Minimum score drop to qualify as a cliff |
| `min_prepoint_duration` | 10 | Frames of high score required before a cliff |
| `min_post_duration` | 10 | Frames of low score required after a cliff |
| `max_post_proba` | 0.55 | Maximum median score allowed in the post window |
| `absolute_threshold` | 0.5 | Maximum score allowed on the frame immediately after |
| `min_gap` | 20 | Minimum frames between consecutive cliffs |
| `smoothing_window` | 3 | Rolling average window applied before detection |
| `video_start_prepoint_threshold` | 0.5 | Relaxed pre-plateau threshold for early-video points |
