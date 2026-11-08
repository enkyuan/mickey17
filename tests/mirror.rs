use axum::{Router, extract::Query, http::HeaderMap, routing::get};
use std::collections::{BTreeSet, HashMap};
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mirror_preserves_html_rsc_and_reports_asset_collisions() {
    const HTML: &str = r#"<h1>Acme</h1><script>self.__next_f.push([1,"0:null\n"])</script><img src="/image.svg?v=1"><img src="/image.svg?v=2">"#;
    let app = Router::new()
        .route(
            "/",
            get(|headers: HeaderMap| async move {
                if headers.get("RSC").is_some_and(|v| v == "1") {
                    ([("content-type", "text/x-component")], "0:null\n")
                } else {
                    ([("content-type", "text/html")], HTML)
                }
            }),
        )
        .route(
            "/image.svg",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                q.get("v").cloned().unwrap_or_default()
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let report = tokio::task::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let report = mochi::mirror::mirror(&client, &[base], BTreeSet::new(), &root).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("index.html")).unwrap(),
            HTML
        );
        assert_eq!(
            std::fs::read_to_string(root.join("index.rsc")).unwrap(),
            "0:null\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("image.svg")).unwrap(),
            "1"
        );
        report
    })
    .await
    .unwrap();
    assert_eq!(report.saved_pages, 1);
    assert_eq!(report.assets_fetched, 1);
    assert_eq!(report.failures.len(), 1);
    assert!(
        report.failures[0]["error"]
            .as_str()
            .unwrap()
            .contains("collision")
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn page_alias_collision_keeps_first_page() {
    let app = Router::new()
        .route("/about", get(|| async { "first page" }))
        .route("/about/index.html", get(|| async { "second page" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let report = tokio::task::spawn_blocking(move || {
        let client = reqwest::blocking::Client::new();
        let pages = [
            base.join("about").unwrap(),
            base.join("about/index.html").unwrap(),
        ];
        let r = mochi::mirror::mirror(&client, &pages, BTreeSet::new(), &root).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("about/index.html")).unwrap(),
            "first page"
        );
        r
    })
    .await
    .unwrap();
    assert_eq!(report.saved_pages, 1);
    assert_eq!(report.failures.len(), 1);
    assert!(
        report.failures[0]["error"]
            .as_str()
            .unwrap()
            .contains("collision")
    );
    server.abort();
}
