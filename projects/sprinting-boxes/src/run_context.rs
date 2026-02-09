use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VideoMetadata {
    pub original_name: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub run_id: String,
    pub team_size: u32,
    pub light_team_name: String,
    pub dark_team_name: String,
    pub tags: Vec<String>,
    #[serde(skip)]
    pub output_dir: PathBuf,
}

impl VideoMetadata {
    /// Creates a new `VideoMetadata` instance with default values.
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

    /// Returns the directory where calibration frames are stored.
    pub fn get_calibration_frames_dir(&self) -> PathBuf {
        self.output_dir.join("calibration_frames")
    }

    /// Extracts calibration frames from the source video.
    pub fn extract_calibration_frames(&self, video_root: &Path) -> Result<Vec<PathBuf>> {
        // Defensive check: if original_name already contains the video_root, don't join it again.
        // This handles existing "doubled" paths in metadata.json gracefully.
        let video_path = if Path::new(&self.original_name).is_absolute() {
            PathBuf::from(&self.original_name)
        } else {
            video_root.join(&self.original_name)
        };

        // Final safety: if the joined path doesn't exist but the relative part does exist inside video_root
        // (handles the case where video_root might be different now)
        let final_path = if !video_path.exists() {
            let filename = Path::new(&self.original_name)
                .file_name()
                .unwrap_or_default();
            video_root.join(filename)
        } else {
            video_path
        };

        let output_dir = self.get_calibration_frames_dir();

        crate::video::calibration::extract_calibration_frames(
            final_path.to_str().unwrap(),
            "opencv", // Default backend
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

        deps
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RunDependency {
    pub artifact_name: String,
    pub message: String,
    pub valid: bool,
}

/// Returns the full path to a specific artifact for a given run.
#[allow(dead_code)]
pub fn get_video_artifact_path(metadata: &VideoMetadata, artifact_name: &str) -> PathBuf {
    metadata.output_dir.join(artifact_name)
}

/// Lists all MP4 video files within the specified root directory, returning paths relative to video_root.
pub fn list_videos(video_root: &Path) -> Vec<PathBuf> {
    WalkDir::new(video_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase() == "mp4")
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

/// Initializes a new analysis run for the given video file.
pub fn create_run(output_root: &Path, video_name: &str) -> Result<VideoMetadata> {
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

    let metadata = VideoMetadata::new(video_name, stem, output_dir);
    metadata.save()?;

    Ok(metadata)
}

/// Creates a new file for a video artifact and returns the file handle.
#[allow(dead_code)]
pub fn create_video_artifact_file(
    metadata: &VideoMetadata,
    artifact_name: &str,
) -> Result<Option<fs::File>> {
    let path = get_video_artifact_path(metadata, artifact_name);
    Ok(Some(fs::File::create(path)?))
}

/// Scans the output root for existing runs and returns their metadata.
pub fn list_runs(output_root: &Path) -> Result<Vec<(String, VideoMetadata)>> {
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
                let mut metadata: VideoMetadata = serde_json::from_str(&content)?;
                metadata.output_dir = path.clone();
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                outputs.push((name, metadata));
            }
        }
    }

    Ok(outputs)
}
