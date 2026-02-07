use axum::{
    body::Body,
    http::{header, HeaderValue, Response, StatusCode},
    response::IntoResponse,
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../sb-dashboard/dist/"]
pub struct Assets;

pub async fn static_handler(path: axum::extract::Path<String>) -> impl IntoResponse {
    let path = path.trim_start_matches('/');

    if path.is_empty() || path == "index.html" {
        return serve_asset("index.html").into_response();
    }

    match serve_asset(path) {
        Ok(response) => response.into_response(),
        Err(_) => serve_asset("index.html")
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::from("404 Not Found"))
                    .unwrap()
            })
            .into_response(),
    }
}

pub async fn index_handler() -> impl IntoResponse {
    serve_asset("index.html").unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(
                "Dashboard not built. Run 'npm run build' in projects/sb-dashboard.",
            ))
            .unwrap()
    })
}

fn serve_asset(path: &str) -> Result<Response<Body>, StatusCode> {
    if let Some(asset) = Assets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();

        Response::builder()
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime.as_ref()).unwrap(),
            )
            .body(Body::from(asset.data.into_owned()))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}
