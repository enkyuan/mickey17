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
        crawl::clone_with_options(
            &format!("http://{addr}"),
            &copy,
            &crawl::Options {
                max_pages: 5,
                extract: true,
                keep_trackers: false,
                ignore_robots: false,
            },
            &crawl::PipelineOptions {
                analyze: true,
                jobs: 2,
                cache_dir: None,
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
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("analysis/report.json")).unwrap()).unwrap();
    assert_eq!(report["pages"].as_array().unwrap().len(), 2);
    let dom: serde_json::Value = serde_json::from_slice(
        &std::fs::read(path.join("analysis/pages/page-00000.json")).unwrap(),
    )
    .unwrap();
    assert!(
        dom["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["tag"] == "img")
    );
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires installed Chrome; run cargo test --test browser -- --ignored"]
async fn rendered_dom_templates_and_cached_assets_work_across_clones() {
    use axum::{
        Router,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    let page = |product: &str| {
        format!(
            r##"<!doctype html><title>Product {product}</title><body><header><h1>Shop</h1><nav><a href="/">First</a><a href="/second">Second</a></nav></header><main><article><h2>{product}</h2><p>Price {product}</p><img src="/logo.svg"><svg><script type="application/json">ignored</script><rect width="5" height="5"/></svg><a href="#buy">Details</a></article></main><script src="/app.js"></script>"##
        )
    };
    let app = Router::new()
        .route("/",get(move || {let html=page("A"); async move {axum::response::Html(html)}}))
        .route("/second",get(move || {let html=page("B"); async move {axum::response::Html(html)}}))
        .route("/robots.txt",get(||async{"User-agent: *\nAllow: /\n"}))
        .route("/favicon.ico",get(||async{vec![0u8,0,1,0,0,0]}))
        .route("/app.js",get(|headers:HeaderMap|async move {
            if headers.get("if-none-match").is_some_and(|value|value=="\"script\"") {
                (StatusCode::NOT_MODIFIED,[("etag","\"script\"")],String::new()).into_response()
            } else {
                (StatusCode::OK,[("etag","\"script\""),("content-type","application/javascript")],"document.querySelector('h2').textContent = 'Rendered ' + document.querySelector('h2').textContent".to_string()).into_response()
            }
        }))
        .route("/logo.svg",get(|headers:HeaderMap|async move {
            if headers.get("if-none-match").is_some_and(|value|value=="\"logo\"") {
                (StatusCode::NOT_MODIFIED,[("etag","\"logo\"")],String::new()).into_response()
            } else {
                (StatusCode::OK,[("etag","\"logo\""),("content-type","image/svg+xml")],r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><rect width="20" height="20"/></svg>"#.to_string()).into_response()
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let options = crawl::Options {
            max_pages: 2,
            extract: false,
            keep_trackers: false,
            ignore_robots: false,
        };
        let pipeline = crawl::PipelineOptions {
            analyze: true,
            jobs: 2,
            cache_dir: Some(dir.join("cache")),
        };
        let first =
            crawl::clone_with_options(&url, &dir.join("first"), &options, &pipeline).unwrap();
        // Editing a clone must never mutate the shared cached origin body.
        std::fs::write(dir.join("first/site/app.js"), "// local user edit").unwrap();
        let second =
            crawl::clone_with_options(&url, &dir.join("second"), &options, &pipeline).unwrap();
        assert!(first.failures.is_empty(), "{:?}", first.failures);
        assert!(second.failures.is_empty(), "{:?}", second.failures);
        assert_eq!(first.cache_revalidations, 0);
        assert_eq!(second.cache_revalidations, 2);
        let root = dir.join("second");
        let report = mochi::analysis::analyze(&root).unwrap();
        assert_eq!(report.templates.len(), 1);
        assert_eq!(report.templates[0].pages.len(), 2);
        assert!(
            report
                .component_candidates
                .iter()
                .any(|candidate| candidate.tag == "article" && candidate.occurrences == 2)
        );
        let snapshot: mochi::analysis::Snapshot = serde_json::from_slice(
            &std::fs::read(root.join("analysis/pages/page-00000.json")).unwrap(),
        )
        .unwrap();
        assert!(
            snapshot
                .nodes
                .iter()
                .any(|node| node.tag == "h2" && node.text == "Rendered A")
        );
        assert!(snapshot.nodes.iter().all(|node| node.tag != "script"));
        assert!(
            snapshot
                .nodes
                .iter()
                .any(|node| node.tag == "rect" && node.namespace == "http://www.w3.org/2000/svg")
        );
        assert!(
            !std::fs::read_to_string(root.join("site/index.html"))
                .unwrap()
                .contains("Rendered A")
        );
        assert!(root.join("site/app.js").exists());
    })
    .await
    .unwrap();
    server.abort();
}
