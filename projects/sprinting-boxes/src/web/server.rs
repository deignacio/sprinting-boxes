use crate::cli::Args;
use crate::web::api::{
    create_run_handler, extract_calibration_frames_handler, get_calibration_frames_handler,
    get_run_handler, get_runs, get_videos, save_boundaries_handler, save_game_details_handler,
    serve_calibration_frame_handler, update_run_handler,
};
use crate::web::assets::{index_handler, static_handler};
use anyhow::Result;
use axum::{
    routing::{get, post, put},
    Router,
};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use tracing::{info, warn};

pub async fn run_server(args: Args) -> Result<()> {
    let host = args.host;
    let port = args.port;
    let shared_args = Arc::new(args);

    let mut current_port = port;
    let listener = loop {
        let addr = SocketAddr::new(host, current_port);
        match TcpListener::bind(addr) {
            Ok(listener) => {
                // FIX: Set non-blocking before registering with Tokio
                listener.set_nonblocking(true)?;
                info!("Successfully bound to {}", addr);
                break listener;
            }
            Err(e) => {
                warn!("Failed to bind to {}: {}. Trying next port...", addr, e);
                current_port += 1;
                if current_port == 0 {
                    return Err(anyhow::anyhow!("No available ports found"));
                }
            }
        }
    };

    let app = Router::new()
        .route("/api/videos", get(get_videos))
        .route("/api/runs", get(get_runs))
        .route("/api/runs", post(create_run_handler))
        .route("/api/runs/:id", get(get_run_handler))
        .route("/api/runs/:id", put(update_run_handler))
        .route(
            "/api/runs/:id/calibration/extract",
            post(extract_calibration_frames_handler),
        )
        .route(
            "/api/runs/:id/calibration/list",
            get(get_calibration_frames_handler),
        )
        .route(
            "/api/runs/:id/calibration/frame/:path",
            get(serve_calibration_frame_handler),
        )
        .route(
            "/api/runs/:id/calibration/boundaries",
            post(save_boundaries_handler),
        )
        .route(
            "/api/runs/:id/calibration/game-details",
            post(save_game_details_handler),
        )
        .route("/", get(index_handler))
        .route("/*path", get(static_handler))
        .with_state(shared_args);

    let tokio_listener = tokio::net::TcpListener::from_std(listener)?;
    info!(
        "Sprinting Boxes server started on http://{:?}",
        tokio_listener.local_addr()?
    );

    axum::serve(tokio_listener, app).await?;

    Ok(())
}
