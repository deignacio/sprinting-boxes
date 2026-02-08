use crate::cli::Args;
use crate::run_context::{list_runs, list_videos, VideoMetadata};
use axum::{
    extract::{Path, State},
    Json,
};
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub struct VideoInfo {
    pub name: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct RunInfo {
    pub name: String,
    pub metadata: VideoMetadata,
}

#[derive(serde::Deserialize)]
pub struct CreateRunRequest {
    pub video_path: String,
}

pub async fn get_videos(State(args): State<Arc<Args>>) -> Json<Vec<VideoInfo>> {
    let video_root = std::path::Path::new(&args.video_root);
    let videos = list_videos(video_root);

    let info_list = videos
        .into_iter()
        .map(|video_path| {
            let name = video_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            let path_str = video_path.to_string_lossy().to_string();
            VideoInfo {
                name,
                path: path_str,
            }
        })
        .collect();

    Json(info_list)
}

pub async fn get_runs(State(args): State<Arc<Args>>) -> Json<Vec<RunInfo>> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = list_runs(output_root).unwrap_or_default();

    let info_list = runs
        .into_iter()
        .map(|(name, metadata)| RunInfo { name, metadata })
        .collect();

    Json(info_list)
}

pub async fn create_run_handler(
    State(args): State<Arc<Args>>,
    Json(payload): Json<CreateRunRequest>,
) -> Result<Json<VideoMetadata>, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    match crate::run_context::create_run(output_root, &payload.video_path) {
        Ok(metadata) => Ok(Json(metadata)),
        Err(e) => {
            tracing::error!("Failed to create run: {}", e);
            Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub async fn update_run_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
    Json(mut payload): Json<VideoMetadata>,
) -> Result<Json<VideoMetadata>, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let run_dir = output_root.join(&run_id);

    if !run_dir.exists() {
        return Err(axum::http::StatusCode::NOT_FOUND);
    }

    payload.output_dir = run_dir;
    if let Err(e) = payload.save() {
        tracing::error!("Failed to update run metadata for {}: {}", run_id, e);
        return Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Json(payload))
}
