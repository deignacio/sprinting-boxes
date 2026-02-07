mod cli;
mod web;

use anyhow::Result;
use cli::Args;
use web::server::run_server;

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables from .env if present
    dotenvy::dotenv().ok();

    // Initialize tracing
    tracing_subscriber::fmt::init();

    let args = Args::parse_args();

    run_server(args.host, args.port).await?;

    Ok(())
}
