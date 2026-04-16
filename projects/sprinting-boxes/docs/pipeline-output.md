# Pipeline output directory reference

Documents every file produced by a `sprinting-boxes` run, including format,
schema, and which worker writes it. Intended as the authoritative output contract
for consumers such as `fiftyone-visualization`, the web dashboard, and external
analysis tools.

---

## Pipeline workers

Spawned by `src/pipeline/orchestrator.rs::start_processing`. Data flows through
bounded `crossbeam::channel` queues between stages.

| # | Worker | Source file | In → out | Role |
|---|---|---|---|---|
| 1 | **Reader** | `pipeline/reader.rs` | video file → `RawFrame` | Decodes frames via opencv or ffmpeg backend. Ranges pulled from a shared pool; supports N parallel readers (default 1). |
| 2 | **Crop** | `pipeline/crop.rs` | `RawFrame` → `PreprocessedFrame` | Applies crop configs (overview + optional left/right endzone) and optional CLAHE contrast enhancement. |
| 3 | **Detection** | `pipeline/detection_worker.rs` | `PreprocessedFrame` → `DetectedFrame` | Runs object detection with slicing/tiling; merges endzone detections into overview via NMS. `fast=true` detects only endzones; default detects full overview. |
| 4 | **Feature** | `pipeline/feature.rs` | `DetectedFrame` → finalized frame | Computes per-frame features (counts, CoM, std dev, deltas) and detects "cliff" frames (point-start transitions). Streams rows to `features.csv` / `points.csv`. |
| 5 | **Finalize** | `pipeline/finalize.rs` | frame → disk | Writes `detection_summary.csv`, `detections.json`, and optional `crops/frame_*.jpg` images. Also runs final NMS aggregation. |
| 6 | **Supervisor** | `pipeline/orchestrator.rs` (inline) | — | Monitors `active_*_workers` atomic counters; closes each inter-stage channel once the upstream stage drains; unregisters pipeline on completion. |

Worker count per stage is dynamically scalable via `scale_workers(run_id, stage, delta)`.
Shutdown is coordinated by the supervisor rather than each worker independently.

---

## Output directory layout

Rooted at `SPRINTING_BOXES_OUTPUT_ROOT/<run_id>/`.

```
<run_id>/
├── metadata.json              ← run config (RunContext)
├── field_boundaries.json      ← user-drawn polygons (pre-pipeline)
├── crops.json                 ← computed crop configs (derived from field_boundaries)
├── features.csv               ← per-frame features (Feature worker)
├── points.csv                 ← cliff-frame summary (Feature worker)
├── detection_summary.csv      ← per-frame NMS stats (Finalize worker)
├── detections.json            ← per-frame bbox data (Finalize worker)
├── crops/                     ← optional; only written if SAVE_VISUAL_CROPS=true|1
│   └── frame_{:06}_{region}.jpg
└── calibration_frames/        ← optional; user-triggered, not part of pipeline
    └── frame_{:03}.jpg
```

---

## File schemas

### `metadata.json`

Written once by `RunContext::save()` at run creation.

```json
{
  "original_name": "string (video filename or absolute path)",
  "display_name": "string",
  "created_at": "ISO 8601 datetime",
  "run_id": "string",
  "team_size": "u32 (default 7)",
  "light_team_name": "string",
  "dark_team_name": "string",
  "tags": ["string"],
  "sample_rate": "f64 (default 1.0)",
  "total_frames": "usize",
  "fps": "f64",
  "duration_secs": "f64",
  "youtube_link": "string | null",
  "fuegostats_link": "string | null"
}
```

---

### `field_boundaries.json`

Written by the web API handler when the user saves field polygons. All coordinates
are normalized to `[0, 1]` in full-image space.

```json
{
  "field": [{"x": "f32", "y": "f32"}, ...],
  "left_end_zone": [{"x": "f32", "y": "f32"}, ...],
  "right_end_zone": [{"x": "f32", "y": "f32"}, ...],
  "roi": {
    "x_normalized": "f32",
    "y_normalized": "f32",
    "width_normalized": "f32",
    "height_normalized": "f32"
  }
}
```

---

### `crops.json`

Written once by `RunContext::compute_and_save_crop_configs()` after boundaries are
saved. Bboxes are in pixel coordinates; polygons are normalized `[0, 1]`.

```json
{
  "overview":       { "name": "overview", "bbox": {"x": "f32", "y": "f32", "w": "f32", "h": "f32"}, "original_polygon": [...], "effective_polygon": [...] },
  "left_end_zone":  { "name": "left",     "bbox": {...}, "original_polygon": [...], "effective_polygon": [...] },
  "right_end_zone": { "name": "right",    "bbox": {...}, "original_polygon": [...], "effective_polygon": [...] },
  "left_end_zone_polygon":  [{"x": "f32", "y": "f32"}, ...],
  "right_end_zone_polygon": [{"x": "f32", "y": "f32"}, ...],
  "field_polygon":          [{"x": "f32", "y": "f32"}, ...]
}
```

`left_end_zone` and `right_end_zone` entries may be `null` if not defined.

---

### `features.csv`

Streamed per frame by the Feature worker.

| Column | Type | Description |
|---|---|---|
| `frame_index` | usize | Frame id |
| `left_count` | f32 | Normalized detection count in left endzone |
| `right_count` | f32 | Normalized detection count in right endzone |
| `field_count` | f32 | Normalized detection count in field |
| `pre_point_score` | f32 | Heuristic point-start score `[0, 1]` |
| `is_cliff` | u8 | `1` if this is a point-start transition frame |
| `com_x` | f32 | Normalized center-of-mass x; `-1.0` if unavailable |
| `com_y` | f32 | Normalized center-of-mass y; `-1.0` if unavailable |
| `distribution_std_dev` | f32 | Std dev of detection spread; `-1.0` if unavailable |
| `com_delta_x` | f32 | Change in CoM x from previous frame; `0.0` if unavailable |
| `com_delta_y` | f32 | Change in CoM y from previous frame; `0.0` if unavailable |
| `std_dev_delta` | f32 | Change in std dev from previous frame; `0.0` if unavailable |

---

### `points.csv`

Streamed by the Feature worker. Contains only rows where `is_cliff == 1`.

| Column | Type | Description |
|---|---|---|
| `frame_index` | usize | Frame id |
| `is_cliff` | u8 | Always `1` |
| `left_side_emptied_first` | u8 | `1` if left endzone emptied before right |
| `right_side_emptied_first` | u8 | `1` if right endzone emptied before left |

---

### `detection_summary.csv`

Streamed per frame by the Finalize worker. Records NMS statistics per crop and
per spatial region.

| Column | Type | Description |
|---|---|---|
| `frame_id` | usize | Frame id |
| `overview_original` | usize | Raw detection count before overview NMS |
| `overview_suppressed` | usize | Removed by overview NMS |
| `overview_close_but_kept` | usize | IoU > 0.3 but ≤ threshold — kept |
| `overview_kept` | usize | Final overview count after NMS |
| `left_original` | usize | Raw count before left endzone NMS |
| `left_suppressed` | usize | Removed by left NMS |
| `left_close_but_kept` | usize | Close but kept in left |
| `left_kept` | usize | Final left count after NMS |
| `right_original` | usize | Raw count before right endzone NMS |
| `right_suppressed` | usize | Removed by right NMS |
| `right_close_but_kept` | usize | Close but kept in right |
| `right_kept` | usize | Final right count after NMS |
| `merge_original` | usize | Combined overview + endzone before merge NMS |
| `merge_suppressed` | usize | Removed by merge NMS |
| `merge_close_but_kept` | usize | Close but kept during merge |
| `merge_kept` | usize | Final count after merge NMS |
| `left_region_kept` | usize | Final count within left region polygon |
| `right_region_kept` | usize | Final count within right region polygon |
| `field_region_kept` | usize | Final count within field region polygon |

---

### `detections.json`

Written by the Finalize worker every 25 frames and once at completion.
Detections are in **crop-local pixel coordinates**.

```json
{
  "version": 2,
  "frames": [
    {
      "id": "usize",
      "crops": {
        "overview": {
          "detections": [
            {
              "x": "f32", "y": "f32", "w": "f32", "h": "f32",
              "conf": "f32",
              "in_end_zone": "bool",
              "in_field": "bool"
            }
          ],
          "regions": [
            { "name": "left | right | field", "polygon": [["f32", "f32"], ...] }
          ],
          "source_bbox": null
        },
        "left": {
          "detections": [...],
          "regions": null,
          "source_bbox": { "x": "f32", "y": "f32", "w": "f32", "h": "f32" }
        },
        "right": {
          "detections": [...],
          "regions": null,
          "source_bbox": { "x": "f32", "y": "f32", "w": "f32", "h": "f32" }
        }
      }
    }
  ]
}
```

- `version = 2` — current schema version for forward-compatibility
- `overview` carries `regions` (zone polygons in crop-local coords) and `source_bbox: null`
- Endzone crops carry `source_bbox` (their origin rect in overview pixel space) and `regions: null`
- `in_end_zone` / `in_field` are convenience flags computed at finalize time

#### Detection color scheme

The dashboard colors detections by zone (see `pipeline/finalize.rs::draw_annotations`):

| `in_end_zone` | `in_field` | Color |
|---|---|---|
| `true` | any | Green `#00FF00` |
| `false` | `true` | Blue `#0000FF` |
| `false` | `false` | Red `#FF0000` (out of bounds) |

---

### `crops/frame_{frame_id:06}_{region}.jpg`

Written by the Finalize worker, one file per crop per frame.
Only written when `SAVE_VISUAL_CROPS` env var is `true` or `1` (default: true).

- `frame_id` — zero-padded 6-digit frame index
- `region` — `"overview"`, `"left"`, or `"right"`
- Content — cropped region post-CLAHE at dimensions from `crops.json`
- Encoding — OpenCV default JPEG quality

---

### `calibration_frames/frame_{n:03}.jpg`

Written by `RunContext::extract_calibration_frames` → `video/calibration.rs`.
**Not** part of the main pipeline — triggered independently via web API.

- Starts at 400 s into the video; extracts 5 frames at 1 s intervals (configurable)
- Full-resolution, no cropping, no CLAHE
- Used by the UI for drawing field boundary polygons

---

## Write-time summary

| File | Writer | Cadence |
|---|---|---|
| `metadata.json` | RunContext | Once at run creation |
| `field_boundaries.json` | Web API | Once (user-defined, pre-pipeline) |
| `crops.json` | RunContext | Once after boundaries saved |
| `features.csv` | Feature worker | Per frame (streamed) |
| `points.csv` | Feature worker | Per cliff frame (streamed) |
| `detection_summary.csv` | Finalize worker | Per frame (streamed) |
| `detections.json` | Finalize worker | Every 25 frames + final |
| `crops/*.jpg` | Finalize worker | Per frame × per region (if enabled) |
| `calibration_frames/*.jpg` | Calibration extractor | On-demand, independent of pipeline |

---

## Coordinate spaces

Several coordinate systems are in use across the output files. Getting these wrong
produces silent bugs (silently-clipped or wildly off-screen overlays). This section
defines each space, states which files use it, and gives the transforms between them.

---

### 1. Full-frame normalized `[0, 1]`

**Used by:** `crops.json::overview.bbox`, `crops.json::field_polygon`,
`crops.json::left_end_zone_polygon`, `crops.json::right_end_zone_polygon`.

All coordinates are expressed as fractions of the **full video frame** dimensions:

- `x ∈ [0, 1]` where `1.0` = full frame width
- `y ∈ [0, 1]` where `1.0` = full frame height

`crops.json` bboxes and the three top-level polygon arrays (`field_polygon`,
`left_end_zone_polygon`, `right_end_zone_polygon`) are all in this space.

> **`field_boundaries.json` caveat:** `field_boundaries.json` stores user-drawn
> polygons in the space of the calibration image displayed in the UI. When an ROI
> is active the UI shows only the ROI region, so the stored coordinates are
> _ROI-relative_, not full-frame. `crops.json` has already applied the ROI
> transform and stores the same polygons in global full-frame normalized space.
> **Always use `crops.json` polygons for rendering** — never `field_boundaries.json`
> directly.

---

### 2. Crop-local pixel space

**Used by:** `detections.json` detection `x`, `y`, `w`, `h`.

Pixel coordinates within the saved overview JPEG
(`crops/frame_{id:06}_overview.jpg`). Origin is the top-left corner of the crop;
axes are in pixels, not fractions.

To obtain the pixel dimensions of the crop, read the JPEG file with an image
library (e.g. PIL). **Do not use `crops.json::overview.bbox` for this** — that
bbox is full-frame normalized `[0, 1]`, not pixels.

Transform to crop-local normalized (needed for FiftyOne):
```
norm_x = det.x / crop_w_px
norm_y = det.y / crop_h_px
norm_w = det.w / crop_w_px
norm_h = det.h / crop_h_px
```

---

### 3. Crop-local normalized `[0, 1]`

**Used by:** FiftyOne `fo.Detection` bounding boxes, `fo.Keypoints`, `fo.Polylines`.

Same origin and axes as the overview JPEG, but expressed as fractions of the
JPEG's pixel dimensions.

Transform from full-frame normalized (for boundary polygons):
```
crop_local_x = (full_frame_x - bbox.x) / bbox.w
crop_local_y = (full_frame_y - bbox.y) / bbox.h
```
where `bbox` is `crops.json::overview.bbox` (full-frame normalized).

---

### 4. Center-of-mass reference space (`com_x` / `com_y` in `features.csv`)

**Used by:** `features.csv` columns `com_x`, `com_y`.

The Feature worker (`pipeline/feature.rs`) computes the pixel-space mean of
detection centre-points and then divides by **hardcoded reference dimensions**
(1920 × 1080), not by the actual crop pixel dimensions. This means the values
are _not_ crop-local normalized even though they fall roughly in `[0, 1]` for
standard-definition content.

Transform to crop-local normalized:
```
cx = com_x_csv * 1920.0 / crop_w_px
cy = com_y_csv * 1080.0 / crop_h_px
```

Similarly, `distribution_std_dev` is scaled by `diagonal / 3` where
`diagonal = sqrt(1920² + 1080²) ≈ 2202`. Convert back to pixel std-dev:
```
std_dev_px = std_dev_csv * (diagonal / 3)
```

**Important:** `std_dev_px` is in **reference 1920×1080 pixel space**, not in crop
pixel space. When displaying the standard deviation circle on a crop of different
size, normalize against the reference height, not the crop height.

A sentinel value of `-1.0` is written when no detections are present (CoM is
undefined). Consumers should treat `-1.0` as `null`/`None`.

---

### 5. Coordinate-space summary table

| File / field | Space | Notes |
|---|---|---|
| `crops.json::overview.bbox` | Full-frame normalized | Use to transform crop ↔ full-frame |
| `crops.json::field_polygon` | Full-frame normalized | Already ROI-transformed; use this, not `field_boundaries.json` |
| `crops.json::left/right_end_zone_polygon` | Full-frame normalized | Same |
| `field_boundaries.json` | ROI-relative (UI space) | Avoid — may not equal full-frame normalized |
| `detections.json` `x/y/w/h` | Crop-local pixel | Divide by JPEG pixel dims (read via PIL) |
| `features.csv` `com_x/com_y` | 1920×1080 reference | Multiply by 1920/crop_w, 1080/crop_h |
| `features.csv` `distribution_std_dev` | (diagonal/3)-normalized | Multiply by diagonal/3 to get pixel std-dev |
| FiftyOne `fo.Detection` | Crop-local normalized | All coordinates ∈ [0, 1] |
| FiftyOne `fo.Keypoints` | Crop-local normalized | |
| FiftyOne `fo.Polylines` | Crop-local normalized | |

---

## Source file reference

| File | Responsibility |
|---|---|
| `src/pipeline/orchestrator.rs` | Worker spawning, supervisor logic |
| `src/pipeline/reader.rs` | Frame decoding |
| `src/pipeline/crop.rs` | Cropping + CLAHE |
| `src/pipeline/detection_worker.rs` | Detection + slice NMS |
| `src/pipeline/feature.rs` | `features.csv`, `points.csv` |
| `src/pipeline/finalize.rs` | `detection_summary.csv`, `detections.json`, `crops/*.jpg` |
| `src/pipeline/types.rs` | `CropConfig`, `CompactFrameData`, `NmsStats`, `ProcessingState` |
| `src/run_context.rs` | `metadata.json`, `crops.json`, calibration frames |
| `src/run_artifacts.rs` | Artifact type definitions |
