use crate::cli::Args;
use crate::run_context::{list_runs, list_videos, RunContext};
use axum::{
    extract::{Path, State},
    response::IntoResponse,
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
    pub run_context: RunContext,
}

#[derive(serde::Deserialize)]
pub struct CreateRunRequest {
    pub video_path: String,
}

#[derive(Serialize)]
pub struct RunDetailResponse {
    pub run_id: String,
    pub run_context: RunContext,
    pub missing_dependencies: Vec<crate::run_context::RunDependency>,
}

pub async fn get_run_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
) -> Result<Json<RunDetailResponse>, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = crate::run_context::list_runs(output_root).map_err(|e| {
        tracing::error!("Failed to list runs: {}", e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(axum::http::StatusCode::NOT_FOUND)?;

    let missing_dependencies = run_context.validate_process_run_dependencies();

    Ok(Json(RunDetailResponse {
        run_id,
        run_context,
        missing_dependencies,
    }))
}

pub async fn extract_calibration_frames_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
) -> Result<Json<Vec<String>>, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let video_root = std::path::Path::new(&args.video_root);

    let runs = crate::run_context::list_runs(output_root).map_err(|e| {
        tracing::error!("Failed to list runs: {}", e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(axum::http::StatusCode::NOT_FOUND)?;

    match run_context.extract_calibration_frames(video_root) {
        Ok(paths) => {
            let filenames = paths
                .into_iter()
                .filter_map(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                })
                .collect();
            Ok(Json(filenames))
        }
        Err(e) => {
            tracing::error!("Failed to extract calibration frames: {}", e);
            Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub async fn get_calibration_frames_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
) -> Result<Json<Vec<String>>, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = crate::run_context::list_runs(output_root).map_err(|e| {
        tracing::error!("Failed to list runs: {}", e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(axum::http::StatusCode::NOT_FOUND)?;

    let dir = run_context.get_calibration_frames_dir();
    if !dir.exists() {
        return Ok(Json(Vec::new()));
    }

    let mut filenames = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".jpg") {
                    filenames.push(name.to_string());
                }
            }
        }
    }
    filenames.sort();
    Ok(Json(filenames))
}

pub async fn serve_calibration_frame_handler(
    State(args): State<Arc<Args>>,
    Path((run_id, filename)): Path<(String, String)>,
) -> Result<impl axum::response::IntoResponse, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let frame_path = output_root
        .join(run_id)
        .join("calibration_frames")
        .join(filename);

    if !frame_path.exists() {
        return Err(axum::http::StatusCode::NOT_FOUND);
    }

    match std::fs::read(frame_path) {
        Ok(data) => {
            let mut response = data.into_response();
            response
                .headers_mut()
                .insert("Content-Type", "image/jpeg".parse().unwrap());
            Ok(response)
        }
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn save_boundaries_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<bool>, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let boundaries_path = output_root.join(&run_id).join("field_boundaries.json");

    match std::fs::write(
        boundaries_path,
        serde_json::to_string_pretty(&payload).unwrap(),
    ) {
        Ok(_) => Ok(Json(true)),
        Err(e) => {
            tracing::error!("Failed to save field boundaries for {}: {}", run_id, e);
            Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub async fn compute_crops_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
) -> Result<Json<crate::run_context::CropsConfig>, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = crate::run_context::list_runs(output_root).map_err(|e| {
        tracing::error!("Failed to list runs: {}", e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(axum::http::StatusCode::NOT_FOUND)?;

    match run_context.compute_and_save_crop_configs() {
        Ok(crops) => Ok(Json(crops)),
        Err(e) => {
            tracing::error!("Failed to compute crop configs for {}: {}", run_id, e);
            Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub async fn get_crops_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
) -> Result<Json<crate::run_context::CropsConfig>, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let runs = crate::run_context::list_runs(output_root).map_err(|e| {
        tracing::error!("Failed to list runs: {}", e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(axum::http::StatusCode::NOT_FOUND)?;

    match run_context.load_crop_configs() {
        Ok(crops) => Ok(Json(crops)),
        Err(e) => {
            tracing::error!("Failed to load crop configs for {}: {}", run_id, e);
            Err(axum::http::StatusCode::NOT_FOUND)
        }
    }
}

pub async fn save_game_details_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<bool>, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let details_path = output_root.join(&run_id).join("game_details.json");

    match std::fs::write(
        details_path,
        serde_json::to_string_pretty(&payload).unwrap(),
    ) {
        Ok(_) => Ok(Json(true)),
        Err(e) => {
            tracing::error!("Failed to save game details for {}: {}", run_id, e);
            Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
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
        .map(|(name, run_context)| RunInfo { name, run_context })
        .collect();

    Json(info_list)
}

pub async fn create_run_handler(
    State(args): State<Arc<Args>>,
    Json(payload): Json<CreateRunRequest>,
) -> Result<Json<RunContext>, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let video_root = std::path::Path::new(&args.video_root);
    match crate::run_context::create_run(output_root, video_root, &payload.video_path) {
        Ok(run_context) => Ok(Json(run_context)),
        Err(e) => {
            tracing::error!("Failed to create run: {}", e);
            Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub async fn update_run_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
    Json(mut payload): Json<RunContext>,
) -> Result<Json<RunContext>, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let run_dir = output_root.join(&run_id);

    if !run_dir.exists() {
        return Err(axum::http::StatusCode::NOT_FOUND);
    }

    payload.output_dir = run_dir;
    if let Err(e) = payload.save() {
        tracing::error!("Failed to update run context for {}: {}", run_id, e);
        return Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Json(payload))
}

#[derive(serde::Deserialize)]
pub struct UpdateWorkerRequest {
    pub delta: i32,
    pub stage: String,
}

// --- Processing API handlers ---

pub async fn update_worker_count_handler(
    Path(run_id): Path<String>,
    Json(payload): Json<UpdateWorkerRequest>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    if let Some(new_count) =
        crate::pipeline::orchestrator::scale_workers(&run_id, &payload.stage, payload.delta)
    {
        Ok(Json(serde_json::json!({ "active_workers": new_count })))
    } else {
        Err(axum::http::StatusCode::NOT_FOUND)
    }
}

pub async fn start_processing_handler(
    State(args): State<Arc<Args>>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    let output_root = std::path::Path::new(&args.output_root);
    let video_root = std::path::Path::new(&args.video_root);

    let runs = crate::run_context::list_runs(output_root).map_err(|e| {
        tracing::error!("Failed to list runs: {}", e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let (_, run_context) = runs
        .into_iter()
        .find(|(id, _)| id == &run_id)
        .ok_or(axum::http::StatusCode::NOT_FOUND)?;

    // Validate dependencies
    let deps = run_context.validate_process_run_dependencies();
    if deps.iter().any(|d| !d.valid) {
        return Err(axum::http::StatusCode::PRECONDITION_FAILED);
    }

    // Start processing
    match crate::pipeline::orchestrator::start_processing(
        &run_context,
        video_root,
        &args.model_path,
    ) {
        Ok(state) => Ok(Json(state.to_progress_json())),
        Err(e) => {
            tracing::error!("Failed to start processing for {}: {:?}", run_id, e);
            Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub async fn stop_processing_handler(
    Path(run_id): Path<String>,
) -> Result<Json<bool>, axum::http::StatusCode> {
    let stopped = crate::pipeline::orchestrator::stop_processing(&run_id);
    Ok(Json(stopped))
}

pub async fn processing_progress_handler(
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    match crate::pipeline::orchestrator::get_processing_state(&run_id) {
        Some(state) => Ok(Json(state.to_progress_json())),
        None => Err(axum::http::StatusCode::NOT_FOUND),
    }
}

pub async fn processing_progress_sse_handler(
    Path(run_id): Path<String>,
) -> axum::response::Sse<
    impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    use axum::response::sse::Event;
    use std::time::Duration;

    tracing::info!("SSE: Connection request for run_id: {}", run_id);

    let stream = async_stream::stream! {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;

            if let Some(state) = crate::pipeline::orchestrator::get_processing_state(&run_id) {
                let json = state.to_progress_json();
                let is_complete = json["is_complete"].as_bool().unwrap_or(false);

                tracing::debug!("SSE: Yielding progress for {}: {}/{}",
                    run_id, json["frames_processed"], json["total_frames"]);

                yield Ok(Event::default().data(json.to_string()));

                if is_complete {
                    break;
                }
            } else {
                break;
            }
        }
        tracing::info!("SSE: Stream ended for {}", run_id);
    };

    axum::response::Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(1))
            .text("keep-alive"),
    )
}
