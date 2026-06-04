use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RunContext {
    pub original_name: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub run_id: String,
    pub team_size: u32,
    pub light_team_name: String,
    pub dark_team_name: String,
    pub tags: Vec<String>,
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
    #[serde(default)]
    pub total_frames: usize,
    #[serde(default)]
    pub fps: f64,
    #[serde(default)]
    pub duration_secs: f64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub youtube_link: Option<String>,
    #[serde(default)]
    pub fuegostats_link: Option<String>,
    #[serde(skip)]
    pub output_dir: PathBuf,
}

fn default_sample_rate() -> f64 {
    1.0
}

impl RunContext {
    /// Creates a new `RunContext` instance with default values.
    pub fn new(video_name: &str, run_id: &str, output_dir: PathBuf) -> Self {
        Self {
            original_name: video_name.to_string(),
            display_name: run_id.to_string(),
            created_at: Utc::now(),
            run_id: run_id.to_string(),
            team_size: 7,
            light_team_name: "Light".to_string(),
            dark_team_name: "Dark".to_string(),
            tags: Vec::new(),
            sample_rate: 1.0,
            total_frames: 0,
            fps: 30.0,
            duration_secs: 0.0,
            width: 0,
            height: 0,
            youtube_link: None,
            fuegostats_link: None,
            output_dir,
        }
    }

    /// Saves the metadata to `metadata.json` in the output directory.
    pub fn save(&self) -> Result<()> {
        let metadata_path = self.output_dir.join("metadata.json");
        let content = serde_json::to_string_pretty(self)?;
        fs::write(metadata_path, content)?;
        Ok(())
    }

    /// Resolves the absolute path to the video file, handling potential path mismatches.
    pub fn resolve_video_path(&self, video_root: &Path) -> PathBuf {
        let original_path = Path::new(&self.original_name);

        // Strategy 1: Absolute path
        if original_path.is_absolute() {
            return PathBuf::from(&self.original_name);
        }

        // Strategy 2: Join with video_root
        let joined_path = video_root.join(&self.original_name);
        if joined_path.exists() {
            return joined_path;
        }

        // Strategy 3: Try just the filename in video_root
        // This handles cases where original_name includes the video_root prefix redundancy
        if let Some(filename) = original_path.file_name() {
            let filename_path = video_root.join(filename);
            if filename_path.exists() {
                return filename_path;
            }
        }

        // Strategy 4: Try original_name relative to CWD (as fallback if it was stored as relative path)
        if original_path.exists() {
            return original_path.to_path_buf();
        }

        // Default validity: return joined path (let it fail at opener if need be, or for error reporting)
        joined_path
    }

    /// Returns the directory where calibration frames are stored.
    pub fn get_calibration_frames_dir(&self) -> PathBuf {
        self.output_dir.join("calibration_frames")
    }

    /// Extracts calibration frames from the source video.
    pub fn extract_calibration_frames(&self, video_root: &Path) -> Result<Vec<PathBuf>> {
        // Defensive check: if original_name already contains the video_root, don't join it again.
        // This handles existing "doubled" paths in metadata.json gracefully.
        let final_path = self.resolve_video_path(video_root);

        let output_dir = self.get_calibration_frames_dir();

        crate::video::calibration::extract_calibration_frames(
            final_path.to_str().unwrap(),
            "ffmpeg", // Use ffmpeg backend
            &output_dir,
            400.0, // Start extraction at 400s
            5,     // Extract 5 frames
            1.0,   // 1 second interval
        )
    }

    /// Validates that all dependencies needed for processing are present.
    pub fn validate_process_run_dependencies(&self) -> Vec<RunDependency> {
        let mut deps = Vec::new();

        // Check for field_boundaries.json
        let field_boundaries_path = self.output_dir.join("field_boundaries.json");
        let field_boundaries_valid = field_boundaries_path.exists();
        deps.push(RunDependency {
            artifact_name: "field_boundaries.json".to_string(),
            message: if field_boundaries_valid {
                "Field boundaries defined.".to_string()
            } else {
                "Field boundaries must be defined before processing.".to_string()
            },
            valid: field_boundaries_valid,
        });

        // Check for crops.json
        let crops_path = self.output_dir.join("crops.json");
        let crops_valid = crops_path.exists();
        deps.push(RunDependency {
            artifact_name: "crops.json".to_string(),
            message: if crops_valid {
                "Crop configurations generated.".to_string()
            } else {
                "Crop configurations must be generated before processing.".to_string()
            },
            valid: crops_valid,
        });

        deps
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RunDependency {
    pub artifact_name: String,
    pub message: String,
    pub valid: bool,
}

// Re-export artifact types from the dedicated module
pub use crate::run_artifacts::{BBox, CropConfigData, CropsConfig, FieldBoundaries, Point};

impl RunContext {
    /// Loads field boundaries from the run's field_boundaries.json.
    pub fn load_field_boundaries(&self) -> Result<FieldBoundaries> {
        let path = self.output_dir.join("field_boundaries.json");
        let content = fs::read_to_string(&path)?;
        let boundaries: FieldBoundaries = serde_json::from_str(&content)?;
        Ok(boundaries)
    }

    /// Computes crop configs from field boundaries and saves to crops.json.
    pub fn compute_and_save_crop_configs(&self) -> Result<CropsConfig> {
        let boundaries = self.load_field_boundaries()?;

        // Get global polygons for all zones
        let field_global = boundaries.get_global_points(&boundaries.field);
        let left_global = boundaries.get_global_points(&boundaries.left_end_zone);
        let right_global = boundaries.get_global_points(&boundaries.right_end_zone);

        // Parameters
        const CROP_PADDING: f32 = 0.01; // 1% crop padding
        const BUFFER_PCT: f32 = 0.05; // 5% diagonal buffer

        // Left Endzone (Effective)
        let left_buffer_dist = crate::geometry::compute_buffer_distance(&left_global, BUFFER_PCT);
        let left_effective = crate::geometry::compute_effective_endzone_polygon(
            &left_global,
            &field_global,
            left_buffer_dist,
        );

        // Right Endzone (Effective)
        let right_buffer_dist = crate::geometry::compute_buffer_distance(&right_global, BUFFER_PCT);
        let right_effective = crate::geometry::compute_effective_endzone_polygon(
            &right_global,
            &field_global,
            right_buffer_dist,
        );

        // Create a union of all points to find the overview bounding box
        let mut all_points = Vec::new();
        all_points.extend_from_slice(&left_effective);
        all_points.extend_from_slice(&right_effective);
        all_points.extend_from_slice(&field_global);

        let overview_bbox =
            crate::geometry::compute_bbox_with_crop_padding(&all_points, CROP_PADDING)
                .ok_or_else(|| anyhow::anyhow!("Failed to compute overview bbox"))?;

        // Compute per-endzone crop bboxes for high-resolution detection
        let left_ez_bbox =
            crate::geometry::compute_bbox_with_crop_padding(&left_effective, CROP_PADDING)
                .ok_or_else(|| anyhow::anyhow!("Failed to compute left EZ bbox"))?;
        let right_ez_bbox =
            crate::geometry::compute_bbox_with_crop_padding(&right_effective, CROP_PADDING)
                .ok_or_else(|| anyhow::anyhow!("Failed to compute right EZ bbox"))?;

        let crops = CropsConfig {
            overview: CropConfigData {
                name: "overview".to_string(),
                bbox: overview_bbox,
                original_polygon: all_points.clone(), // Union of all original points
                effective_polygon: all_points,        // Same for overview
            },
            left_end_zone: Some(CropConfigData {
                name: "left".to_string(),
                bbox: left_ez_bbox,
                original_polygon: left_global.clone(),
                effective_polygon: left_effective.clone(),
            }),
            right_end_zone: Some(CropConfigData {
                name: "right".to_string(),
                bbox: right_ez_bbox,
                original_polygon: right_global.clone(),
                effective_polygon: right_effective.clone(),
            }),
            left_end_zone_polygon: left_effective,
            right_end_zone_polygon: right_effective,
            field_polygon: field_global,
        };

        let crops_path = self.output_dir.join("crops.json");
        let content = serde_json::to_string_pretty(&crops)?;
        fs::write(crops_path, content)?;

        Ok(crops)
    }

    /// Loads existing crop configs from crops.json.
    pub fn load_crop_configs(&self) -> Result<CropsConfig> {
        let path = self.output_dir.join("crops.json");
        let content = fs::read_to_string(&path)?;
        let crops: CropsConfig = serde_json::from_str(&content)?;
        Ok(crops)
    }
}

/// Lists all video files (MP4, WebM) within the specified root directory, returning paths relative to video_root.
pub fn list_videos(video_root: &Path) -> Vec<PathBuf> {
    WalkDir::new(video_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| {
                    let lower = s.to_lowercase();
                    lower == "mp4" || lower == "webm"
                })
                .unwrap_or(false)
        })
        .filter_map(|e| {
            e.path()
                .strip_prefix(video_root)
                .ok()
                .map(|p| p.to_path_buf())
        })
        .collect()
}

/// Video metadata extracted from a file using ffmpeg-next.
pub(crate) struct VideoMetadata {
    pub total_frames: usize,
    pub fps: f64,
    pub duration_secs: f64,
    pub width: u32,
    pub height: u32,
}

/// Probe video file for metadata using ffmpeg-next.
pub(crate) fn probe_video_metadata_ffmpeg(path: &str) -> Result<VideoMetadata> {
    crate::video::ffmpeg_reader::init_ffmpeg()?;

    let source = Path::new(path);
    let input_ctx = ffmpeg_next::format::input(source)
        .context("ffmpeg: failed to open video for metadata probe")?;

    let video_stream = input_ctx
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .ok_or_else(|| anyhow::anyhow!("ffmpeg: no video stream found in {}", path))?;

    let rational_fps = video_stream.avg_frame_rate();
    let fps = if rational_fps.denominator() > 0 {
        rational_fps.numerator() as f64 / rational_fps.denominator() as f64
    } else {
        0.0
    };

    let stream_frames = video_stream.frames() as usize;
    let duration_secs = input_ctx.duration() as f64 / ffmpeg_next::ffi::AV_TIME_BASE as f64;

    let total_frames = if stream_frames > 0 {
        stream_frames
    } else if fps > 0.0 {
        (duration_secs * fps).round() as usize
    } else {
        0
    };

    // Get width and height from decoder parameters
    let decoder_ctx = ffmpeg_next::codec::context::Context::from_parameters(
        video_stream.parameters(),
    )
    .context("ffmpeg: failed to create decoder context for metadata")?;
    let decoder = decoder_ctx
        .decoder()
        .video()
        .context("ffmpeg: failed to open video decoder for metadata")?;

    Ok(VideoMetadata {
        total_frames,
        fps,
        duration_secs,
        width: decoder.width(),
        height: decoder.height(),
    })
}

/// Initializes a new analysis run for the given video file.
pub fn create_run(output_root: &Path, video_root: &Path, video_name: &str) -> Result<RunContext> {
    let stem = Path::new(video_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid video name: {}", video_name))?;

    let output_dir = output_root.join(stem);
    if output_dir.exists() {
        return Err(anyhow::anyhow!(
            "Output directory already exists for: {}",
            stem
        ));
    }

    fs::create_dir_all(&output_dir)?;

    // Resolve absolute path to video
    let full_path = video_root.join(video_name);
    let absolute_path = std::fs::canonicalize(&full_path).unwrap_or(full_path);
    let absolute_path_str = absolute_path.to_string_lossy();

    let mut run_context = RunContext::new(&absolute_path_str, stem, output_dir);

    // Extract metadata from video using ffmpeg-next
    if let Ok(meta) = probe_video_metadata_ffmpeg(&absolute_path_str) {
        run_context.total_frames = meta.total_frames;
        run_context.fps = meta.fps;
        run_context.duration_secs = meta.duration_secs;
        run_context.width = meta.width;
        run_context.height = meta.height;
    } else {
        tracing::warn!("Failed to probe video metadata via ffmpeg for: {}", absolute_path_str);
    }

    run_context.save()?;

    Ok(run_context)
}

/// Scans the output root for existing runs and returns their metadata.
pub fn list_runs(output_root: &Path) -> Result<Vec<(String, RunContext)>> {
    let mut outputs = Vec::new();

    if !output_root.exists() {
        return Ok(outputs);
    }

    for entry in fs::read_dir(output_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let metadata_path = path.join("metadata.json");
            if metadata_path.exists() {
                let content = fs::read_to_string(metadata_path)?;
                let mut run_context: RunContext = serde_json::from_str(&content)?;
                run_context.output_dir = path.clone();
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                // Sync internal run_id with folder name (Source of Truth for API)
                run_context.run_id = name.clone();

                outputs.push((name, run_context));
            }
        }
    }

    Ok(outputs)
}
