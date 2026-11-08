use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::IntoResponse,
    routing::get,
};
use mochi::cache::AssetCache;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Default)]
struct Origin {
    version: AtomicUsize,
    policy: AtomicUsize,
    unavailable: AtomicBool,
    requests: Mutex<Vec<(String, bool)>>,
}

async fn asset(
    State(origin): State<Arc<Origin>>,
    uri: Uri,
    headers: HeaderMap,
) -> axum::response::Response {
    origin
        .requests
        .lock()
        .unwrap()
        .push((uri.to_string(), headers.contains_key("if-none-match")));
    if origin.unavailable.load(Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let version = origin.version.load(Ordering::SeqCst);
    let query = uri.query().unwrap_or("");
    let etag = format!("\"v{version}-{query}\"");
    let mut response_headers = HeaderMap::new();
    response_headers.insert("etag", etag.parse().unwrap());
    response_headers.insert("content-type", "application/javascript".parse().unwrap());
    let policy = origin.policy.load(Ordering::SeqCst);
    response_headers.insert(
        "cache-control",
        match policy {
            1 => "no-store",
            2 => "private",
            _ => "public, max-age=0",
        }
        .parse()
        .unwrap(),
    );
    if policy == 3 {
        response_headers.insert("set-cookie", "session=fixture".parse().unwrap());
    }
    if policy == 4 {
        response_headers.insert("vary", "Accept-Language".parse().unwrap());
    }
    if headers
        .get("if-none-match")
        .is_some_and(|value| value == etag.as_str())
    {
        (StatusCode::NOT_MODIFIED, response_headers, String::new()).into_response()
    } else {
        (
            StatusCode::OK,
            response_headers,
            format!("body-{version}-{query}"),
        )
            .into_response()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_revalidates_preserves_queries_and_deduplicates_identical_bodies() {
    let origin = Arc::new(Origin::default());
    let app = Router::new()
        .route("/{*path}", get(asset))
        .with_state(origin.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    let origin_for_test = origin.clone();
    tokio::task::spawn_blocking(move || {
        let cache = AssetCache::open(tmp.path().into()).unwrap();
        let client = reqwest::blocking::Client::new();
        let a = base.join("a.js").unwrap();
        assert!(!cache.fetch(&client, &a).unwrap().reused);
        assert!(
            !cache
                .fetch(&client, &base.join("b.js").unwrap())
                .unwrap()
                .reused
        );
        assert_eq!(tmp.path().join("objects").read_dir().unwrap().count(), 1);
        assert!(cache.fetch(&client, &a).unwrap().reused);
        origin_for_test.version.store(1, Ordering::SeqCst);
        let changed = cache.fetch(&client, &a).unwrap();
        assert!(!changed.reused);
        assert_eq!(changed.bytes, b"body-1-");
        assert!(cache.fetch(&client, &a).unwrap().reused);
        let red = base.join("a.js?v=red").unwrap();
        let blue = base.join("a.js?v=blue").unwrap();
        assert_eq!(cache.fetch(&client, &red).unwrap().bytes, b"body-1-v=red");
        assert_eq!(cache.fetch(&client, &blue).unwrap().bytes, b"body-1-v=blue");
        assert!(cache.fetch(&client, &red).unwrap().reused);
        let requests = origin_for_test.requests.lock().unwrap();
        assert_eq!(requests.len(), 8);
        assert!(!requests[5].1 && !requests[6].1 && requests[7].1);
    })
    .await
    .unwrap();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corrupt_or_forbidden_cache_entries_are_not_reused_and_origin_errors_are_not_hidden() {
    let origin = Arc::new(Origin::default());
    let app = Router::new()
        .route("/{*path}", get(asset))
        .with_state(origin.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = url::Url::parse(&format!("http://{}/a.js", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    tokio::task::spawn_blocking(move || {
        let cache = AssetCache::open(tmp.path().into()).unwrap();
        let client = reqwest::blocking::Client::new();
        cache.fetch(&client, &url).unwrap();
        let object = tmp
            .path()
            .join("objects")
            .read_dir()
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::write(&object, "corrupted").unwrap();
        let repaired = cache.fetch(&client, &url).unwrap();
        assert!(!repaired.reused);
        assert_eq!(repaired.bytes, b"body-0-");
        assert_eq!(std::fs::read(object).unwrap(), b"body-0-");
        for policy in 1..=4 {
            origin.policy.store(policy, Ordering::SeqCst);
            let response = cache.fetch(&client, &url).unwrap();
            assert!(!response.reused);
            assert_eq!(response.bytes, b"body-0-");
            assert_eq!(tmp.path().join("urls").read_dir().unwrap().count(), 0);
            origin.policy.store(0, Ordering::SeqCst);
            assert!(!cache.fetch(&client, &url).unwrap().reused);
        }
        origin.unavailable.store(true, Ordering::SeqCst);
        assert!(cache.fetch(&client, &url).is_err());
    })
    .await
    .unwrap();
    server.abort();
}

#[cfg(unix)]
#[test]
fn cache_refuses_symlinked_object_storage() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("objects")).unwrap();
    assert!(AssetCache::open(root.path().into()).is_err());
    assert_eq!(outside.path().read_dir().unwrap().count(), 0);
}

// Value: protects=recursive asset discovery obeys worker bounds; fails_when=child batches are lost or concurrency exceeds jobs; why_new=previous fixture had independent assets; seam=none; no_claim=arbitrary JavaScript discovery
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_downloads_obey_the_worker_limit() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let a = active.clone();
    let p = peak.clone();
    let app = Router::new()
        .route("/", get(|| async { "<h1>Fixture</h1>" }))
        .route("/child.png", get(|| async { "child bytes" }))
        .route(
            "/assets/{name}",
            get(move || {
                let active = a.clone();
                let peak = p.clone();
                async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    "body{background:url(/child.png)}"
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    tokio::task::spawn_blocking(move || {
        let client = reqwest::blocking::Client::new();
        let assets = (0..6)
            .map(|n| base.join(&format!("assets/{n}.css")).unwrap())
            .collect();
        let report = mochi::mirror::mirror_with_options(
            &client,
            &[base],
            assets,
            tmp.path(),
            &mochi::mirror::DownloadOptions {
                jobs: 2,
                cache_dir: None,
            },
        )
        .unwrap();
        assert_eq!(report.assets_fetched, 7);
        assert_eq!(
            std::fs::read(tmp.path().join("child.png")).unwrap(),
            b"child bytes"
        );
        assert!(report.failures.is_empty(), "{:?}", report.failures);
    })
    .await
    .unwrap();
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn validators_do_not_cross_redirect_targets_and_last_modified_is_supported() {
    let redirect = Arc::new(AtomicBool::new(false));
    let route_state = redirect.clone();
    let app = Router::new()
        .route(
            "/entry.js",
            get(move || {
                let redirect = route_state.clone();
                async move {
                    if redirect.load(Ordering::SeqCst) {
                        (StatusCode::FOUND, [("location", "/target.js")], "").into_response()
                    } else {
                        (StatusCode::OK, [("etag", "\"shared\"")], "first").into_response()
                    }
                }
            }),
        )
        .route(
            "/target.js",
            get(|headers: HeaderMap| async move {
                if headers.contains_key("if-none-match") {
                    (StatusCode::NOT_MODIFIED, "").into_response()
                } else {
                    (
                        StatusCode::OK,
                        [("etag", "\"shared\"")],
                        "different resource",
                    )
                        .into_response()
                }
            }),
        )
        .route(
            "/modified.css",
            get(|headers: HeaderMap| async move {
                let modified = "Wed, 21 Oct 2015 07:28:00 GMT";
                if headers
                    .get("if-modified-since")
                    .is_some_and(|value| value == modified)
                {
                    (
                        StatusCode::NOT_MODIFIED,
                        [("last-modified", modified), ("content-type", "text/plain")],
                        "",
                    )
                        .into_response()
                } else {
                    (
                        StatusCode::OK,
                        [("last-modified", modified), ("content-type", "text/css")],
                        "body{}",
                    )
                        .into_response()
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    tokio::task::spawn_blocking(move || {
        let cache = AssetCache::open(root.path().into()).unwrap();
        let client = reqwest::blocking::Client::new();
        let url = base.join("entry.js").unwrap();
        assert_eq!(cache.fetch(&client, &url).unwrap().bytes, b"first");
        redirect.store(true, Ordering::SeqCst);
        let changed = cache.fetch(&client, &url).unwrap();
        assert!(!changed.reused);
        assert_eq!(changed.bytes, b"different resource");
        let modified = base.join("modified.css").unwrap();
        assert!(!cache.fetch(&client, &modified).unwrap().reused);
        let reused = cache.fetch(&client, &modified).unwrap();
        assert!(reused.reused);
        assert_eq!(reused.bytes, b"body{}");
        // A 304 can update representation metadata without transferring its body.
        assert_eq!(reused.content_type, "text/plain");
    })
    .await
    .unwrap();
    server.abort();
}

// Value: protects=cache credential isolation and recovery under concurrent writers; fails_when=credentials persist or malformed metadata blocks fresh bytes; why_new=prior tests cover object corruption only; seam=none; no_claim=process-crash durability
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credentials_metadata_and_concurrent_writes_preserve_origin_bytes() {
    let origin = Arc::new(Origin::default());
    let app = Router::new()
        .route("/{*path}", get(asset))
        .with_state(origin.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = url::Url::parse(&format!("http://{}/a.js", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    tokio::task::spawn_blocking(move || {
        let cache = AssetCache::open(root.path().into()).unwrap();
        let client = reqwest::blocking::Client::new();
        let mut credentialed = url.clone();
        credentialed.set_username("user").unwrap();
        credentialed.set_password(Some("secret")).unwrap();
        for _ in 0..2 {
            let response = cache.fetch(&client, &credentialed).unwrap();
            assert!(!response.reused);
            assert_eq!(response.bytes, b"body-0-");
        }
        assert_eq!(root.path().join("urls").read_dir().unwrap().count(), 0);
        cache.fetch(&client, &url).unwrap();
        let entry = root
            .path()
            .join("urls")
            .join(mochi::cache::fingerprint(url.as_str().as_bytes()));
        let original = std::fs::read(&entry).unwrap();
        let mut invalid: serde_json::Value = serde_json::from_slice(&original).unwrap();
        invalid["etag"] = serde_json::json!("bad\nheader");
        for bytes in [
            b"not json".to_vec(),
            vec![b' '; 16 * 1024 + 1],
            serde_json::to_vec(&invalid).unwrap(),
        ] {
            std::fs::write(&entry, bytes).unwrap();
            let response = cache.fetch(&client, &url).unwrap();
            assert!(!response.reused);
            assert_eq!(response.bytes, b"body-0-");
        }
        std::fs::remove_file(&entry).unwrap();
        let barrier = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let (cache, client, url, barrier) = (&cache, &client, &url, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    assert_eq!(cache.fetch(client, url).unwrap().bytes, b"body-0-");
                });
            }
        });
        let response = cache.fetch(&client, &url).unwrap();
        assert!(response.reused);
        assert_eq!(response.bytes, b"body-0-");
        assert_eq!(root.path().join("objects").read_dir().unwrap().count(), 1);
    })
    .await
    .unwrap();
    server.abort();
}
