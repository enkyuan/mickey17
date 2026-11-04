use crate::util;
use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
};
use std::path::PathBuf;
use tower::ServiceExt;
use tower_http::services::ServeFile;

async fn file(State(root): State<PathBuf>, mut req: Request) -> Response {
    let uri = req.uri().clone();
    let url = match url::Url::parse(&format!("http://localhost{uri}")) {
        Ok(u) => u,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let mut path = url.path().to_string();
    if path == "/_next/image"
        && let Some((_, src)) = url.query_pairs().find(|(k, _)| k == "url")
    {
        if let Ok(remote) = url::Url::parse(&src)
            && matches!(remote.scheme(), "http" | "https")
        {
            return (StatusCode::FOUND, [("location", remote.as_str())]).into_response();
        }
        if src.starts_with('/') {
            let Ok(image) = url.join(&src) else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            if image.host_str() != url.host_str() {
                return StatusCode::BAD_REQUEST.into_response();
            }
            path = image.path().to_string()
        } else {
            return StatusCode::BAD_REQUEST.into_response();
        }
    }
    let Ok(mut local) = util::contained(&root, &path) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if (url.query_pairs().any(|(k, _)| k == "_rsc")
        || req.headers().get("RSC").is_some_and(|h| h == "1"))
        && local.join("index.rsc").is_file()
    {
        local = local.join("index.rsc");
    } else if local.is_dir() {
        local = local.join("index.html")
    }
    // Recheck final components, including index files, for symlinks.
    if util::checked_path(&root, &local).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    *req.uri_mut() = Uri::from_static("/");
    match ServeFile::new(&local).oneshot(req).await {
        Ok(response) => {
            let (mut parts, body) = response.into_parts();
            if local.extension().is_some_and(|s| s == "rsc") {
                parts.headers.insert(
                    "content-type",
                    axum::http::HeaderValue::from_static("text/x-component"),
                );
            }
            Response::from_parts(parts, Body::new(body))
        }
        Err(e) => match e {},
    }
}
pub fn router(root: PathBuf) -> Router {
    Router::new().fallback(get(file)).with_state(root)
}
pub async fn serve(path: PathBuf, port: u16) -> anyhow::Result<()> {
    let root = util::site_dir(&path).canonicalize()?;
    let mut listener = None;
    for p in port..=port.saturating_add(19) {
        if let Ok(l) = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, p)).await {
            listener = Some(l);
            break;
        }
    }
    let listener = listener.ok_or_else(|| anyhow::anyhow!("no free preview port"))?;
    println!(
        "Serving {} at http://{}",
        root.display(),
        listener.local_addr()?
    );
    axum::serve(listener, router(root))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
