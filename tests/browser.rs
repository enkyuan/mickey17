//! Deterministic Chromium smoke test. Requires Chrome and loopback networking.
use mochi::{crawl, edit, serve};
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires installed Chrome; run cargo test --test browser -- --ignored"]
async fn clone_render_extract_edit_and_serve() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("favicon.ico"), [0, 0, 1, 0, 0, 0]).unwrap();
    std::fs::write(root.join("index.html"),r#"<!doctype html><title>Acme</title><h1>Acme</h1><script src="/app.js"></script><a href="/about">About</a><a href="/private">Private</a>"#).unwrap();
    std::fs::create_dir(root.join("about")).unwrap();
    std::fs::write(
        root.join("about/index.html"),
        "<!doctype html><title>About</title><h1>Acme about</h1>",
    )
    .unwrap();
    std::fs::write(
        root.join("app.js"),
        "for(let i=0;i<3;i++)fetch('/pending');setTimeout(()=>{const i=new Image();i.src='/lazy.svg';document.body.append(i)},100)",
    )
    .unwrap();
    std::fs::write(
        root.join("lazy.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg"><text>Acme</text></svg>"#,
    )
    .unwrap();
    std::fs::write(
        root.join("robots.txt"),
        "User-agent: *\nDisallow: /private\n",
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = serve::router(fixture.path().into()).route(
            "/pending",
            axum::routing::get(|| async { std::future::pending::<&'static str>().await }),
        );
        axum::serve(listener, app).await.unwrap();
        drop(fixture)
    });
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("clone");
    let copy = path.clone();
    let report = tokio::task::spawn_blocking(move || {
        crawl::clone_site(
            &format!("http://{addr}"),
            &copy,
            &crawl::Options {
                max_pages: 5,
                extract: true,
                keep_trackers: false,
                ignore_robots: false,
            },
        )
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(report.saved_pages, 2);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert!(path.join("site/lazy.svg").exists());
    assert!(path.join("extract/page-00000.png").exists());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("manifest.json")).unwrap()).unwrap();
    assert!(
        manifest["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["reason"] == "robots.txt")
    );
    let r = edit::Replacer::new("Acme", "Globex", true, true).unwrap();
    edit::apply(&path.join("site"), &|s| r.replace(s), &|s| r.replace(s)).unwrap();
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let res = serve::router(path.join("site"))
        .oneshot(
            axum::http::Request::builder()
                .uri("/about")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("Globex about"));
    server.abort();
}
