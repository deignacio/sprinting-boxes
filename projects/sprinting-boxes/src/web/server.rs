use crate::web::assets::{index_handler, static_handler};
use anyhow::Result;
use axum::{routing::get, Router};
use std::net::{IpAddr, SocketAddr, TcpListener};
use tracing::{info, warn};

pub async fn run_server(host: IpAddr, port: u16) -> Result<()> {
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
        .route("/", get(index_handler))
        .route("/*path", get(static_handler));

    let tokio_listener = tokio::net::TcpListener::from_std(listener)?;
    info!(
        "Sprinting Boxes server started on http://{:?}",
        tokio_listener.local_addr()?
    );

    axum::serve(tokio_listener, app).await?;

    Ok(())
}
