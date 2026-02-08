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
}

/// Returns the full path to a specific artifact for a given run.
#[allow(dead_code)]
pub fn get_video_artifact_path(metadata: &VideoMetadata, artifact_name: &str) -> PathBuf {
    metadata.output_dir.join(artifact_name)
}

/// Lists all MP4 video files within the specified root directory.
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
        .map(|e| e.path().to_path_buf())
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
