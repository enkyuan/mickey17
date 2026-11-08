use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;
async fn get(
    root: &std::path::Path,
    uri: &str,
    header: Option<(&str, &str)>,
) -> (u16, axum::http::HeaderMap, Vec<u8>) {
    let mut req = Request::builder().uri(uri);
    if let Some((k, v)) = header {
        req = req.header(k, v)
    }
    let res = mosha::serve::router(root.to_path_buf())
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (parts, body) = res.into_parts();
    (
        parts.status.as_u16(),
        parts.headers,
        body.collect().await.unwrap().to_bytes().to_vec(),
    )
}
#[tokio::test]
async fn routes_ranges_head_and_symlinks() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    std::fs::create_dir(p.join("about")).unwrap();
    std::fs::write(p.join("about/index.html"), "about page").unwrap();
    std::fs::write(p.join("about/index.rsc"), "0:null\n").unwrap();
    std::fs::write(p.join("hero.png"), b"png").unwrap();
    assert_eq!(get(p, "/about", None).await.2, b"about page");
    let (code, h, b) = get(p, "/about?_rsc=x", None).await;
    assert_eq!(code, 200);
    assert_eq!(h["content-type"], "text/x-component");
    assert_eq!(b, b"0:null\n");
    assert_eq!(get(p, "/about", Some(("RSC", "1"))).await.2, b"0:null\n");
    assert_eq!(
        get(p, "/_next/image?url=%2Fhero.png&w=640", None).await.2,
        b"png"
    );
    let (code, h, _) = get(p, "/_next/image?url=https%3A%2F%2Fcdn.test%2Fa.png", None).await;
    assert_eq!(code, 302);
    assert_eq!(h["location"], "https://cdn.test/a.png");
    for range in ["bytes=1-2", "bytes=-2"] {
        let (code, h, b) = get(p, "/hero.png", Some(("Range", range))).await;
        assert_eq!(code, 206);
        assert_eq!(h["content-range"], "bytes 1-2/3");
        assert_eq!(b, b"ng");
    }
    assert_eq!(
        get(p, "/hero.png", Some(("Range", "bytes=9-"))).await.0,
        416
    );
    assert_eq!(get(p, "/absent", None).await.0, 404);
    let req = Request::builder()
        .method("HEAD")
        .uri("/hero.png")
        .body(Body::empty())
        .unwrap();
    let res = mosha::serve::router(p.into()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), 200);
    assert!(
        res.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", p.join("secret")).unwrap();
        assert_eq!(get(p, "/secret", None).await.0, 403);
    }
}
