//! Real Chromium evidence contracts on owned loopback fixtures. No remote services.
use mochi::{crawl, evidence};
use serde_json::json;

fn launch() -> headless_chrome::Browser {
    let mut options = headless_chrome::LaunchOptions::default_builder();
    options.window_size(Some((1440, 900)));
    if let Some(path) = std::env::var_os("MOCHI_CHROME") {
        options.path(Some(path.into()));
    }
    headless_chrome::Browser::new(options.build().unwrap()).unwrap()
}
fn document() -> String {
    format!(
        r##"<!doctype html><html lang="en"><head><title>Source fixture</title><meta name="description" content="Full evidence"><link rel="stylesheet" href="/style.css"></head><body><p id="mixed">{}<em>inside 🙂</em> after 🙂</p><input id="live" value="original"><input id="checked" type="checkbox" checked><textarea id="area">original</textarea><svg xmlns="http://www.w3.org/2000/svg"><use xmlns:xlink="http://www.w3.org/1999/xlink" xlink:href="#shape"/></svg><a href="/?variant=two">Other state</a><script>document.querySelector('#live').value='live value';document.querySelector('#checked').checked=false;document.querySelector('#area').value='live textarea';</script></body></html>"##,
        "before 🙂 ".repeat(100)
    )
}
const CSS: &str = r#"@font-face {font-family: UnusedFixture;src: url('/unused.woff2');font-weight: 400 700;font-style: italic;font-display: swap;unicode-range: U+0000-00FF;} :root {--brand: #123456;} body {color:var(--brand);font-family:sans-serif;} p {font-size:20px;} p::before {content:'prefix';display:inline;} @media (max-width:500px) {p {font-size:12px;}} @supports (display:grid) {body {display:grid;}}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires installed Chrome; run cargo test --test source_browser -- --ignored"]
async fn rich_capture_preserves_text_css_controls_queries_and_viewport_screenshots() {
    use axum::{Router, response::Html, routing::get};
    let app = Router::new()
        .route("/", get(|| async { Html(document()) }))
        .route("/robots.txt", get(|| async { "User-agent: *\nAllow: /\n" }))
        .route("/favicon.ico", get(|| async { vec![0u8] }))
        .route(
            "/style.css",
            get(|| async { ([("content-type", "text/css")], CSS) }),
        )
        .route("/unused.woff2", get(|| async { vec![0u8] }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/?variant=one", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("clone");
    tokio::task::spawn_blocking(move||{
        let report=crawl::clone_with_evidence(&url,&out,&crawl::Options{max_pages:2,extract:false,keep_trackers:false,ignore_robots:false},&crawl::PipelineOptions::default(),&evidence::CaptureOptions{viewports:vec!["375x812".parse().unwrap(),"1440x900".parse().unwrap()]}).unwrap();
        assert_eq!(report.saved_pages,2);assert!(report.failures.is_empty(),"{:?}",report.failures);
        let summary=evidence::inspect(&out).unwrap();assert_eq!(summary.captures.len(),4);assert_eq!(summary.complete,4,"{:?}",summary.captures);
        assert!(summary.captures.iter().any(|c|c.url.ends_with("?variant=two")));
        let project=mochi::project::build(&out).unwrap();
        assert_eq!(project.routes.len(),2);
        assert_eq!(project.observations.len(),4);
        assert!(project.assets.values().any(|a|a.url.ends_with("/style.css") && a.status=="local_candidate" && a.local_path.as_deref()==Some("site/style.css")));
        assert!(project.routes.values().all(|r|r.query.is_some()));
        assert!(project.styles.values().any(|s|s.get("font-size").is_some_and(|v|v=="12px")));
        assert!(project.styles.values().any(|s|s.get("font-size").is_some_and(|v|v=="20px")));
        assert!(project.components.values().any(|c|c.instances.iter().map(|i|&i.style_variant).collect::<std::collections::BTreeSet<_>>().len()>1));
        let first=std::fs::read(out.join("reconstruction/project.json")).unwrap();
        mochi::project::build(&out).unwrap();
        assert_eq!(first,std::fs::read(out.join("reconstruction/project.json")).unwrap());
        for (width,size) in [(375,"12px"),(1440,"20px")] {
            let entry=summary.captures.iter().find(|c|c.viewport.width==width).unwrap();
            let capture=evidence::load(&out,std::path::Path::new(&entry.snapshot)).unwrap();capture.require_complete().unwrap();
            assert!(capture.nodes.iter().any(|n|matches!(&n.content,evidence::Content::Text{value} if value==&"before 🙂 ".repeat(100))));
            let p=capture.nodes.iter().find(|n|matches!(&n.content,evidence::Content::Element{tag,..} if tag=="p")).unwrap();
            if let evidence::Content::Element{style,pseudo,..}=&p.content {
                assert_eq!(capture.styles[*style]["font-size"],size);
                assert_eq!(capture.styles[*style]["--brand"],"#123456");
                assert!(capture.styles[pseudo["::before"]]["content"].contains("prefix"));
            }
            assert!(capture.stylesheets.iter().flat_map(|s|&s.rules).any(|r|r.contains("@media")&&r.contains("500px")));
            assert!(capture.stylesheets.iter().flat_map(|s|&s.rules).any(|r|r.contains("@supports")));
            assert!(capture.fonts.iter().any(|f|f.descriptors.get("font-display").is_some_and(|v|v=="swap")&&f.descriptors.get("font-weight").is_some_and(|v|v=="400 700")));
            assert!(capture.nodes.iter().any(|n|matches!(&n.content,evidence::Content::Element{control:Some(c),..} if c.value.as_deref()==Some("live value"))));
            assert!(capture.nodes.iter().any(|n|matches!(&n.content,evidence::Content::Element{control:Some(c),..} if c.value.as_deref()==Some("live textarea"))));
            assert!(capture.nodes.iter().any(|n|matches!(&n.content,evidence::Content::Element{control:Some(c),..} if c.checked==Some(false))));
            assert!(capture.nodes.iter().any(|n|matches!(&n.content,evidence::Content::Element{attributes,..} if attributes.iter().any(|a|a.namespace.as_deref()==Some("http://www.w3.org/1999/xlink")))));
            assert!(!capture.nodes.iter().any(|n|matches!(&n.content,evidence::Content::Element{tag,..} if tag=="script")));
            let projection=capture.to_analysis_snapshot().unwrap();
            let paragraph=projection.nodes.iter().find(|n|n.tag=="p").unwrap();assert_eq!(paragraph.text_slots,Some(vec![0,1]));
        }
        assert!(!out.join("analysis").exists()); // Source capture does not change opt-in legacy analysis.
        let cli_out=out.parent().unwrap().join("cli-clone");
        let output=std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
            .args(["clone",&url,"--source-evidence","--source-viewports","375x812","--max-pages","1","-o"])
            .arg(&cli_out).output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
        assert_eq!(evidence::inspect(&cli_out).unwrap().complete,1);
    }).await.unwrap();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires installed Chrome; run cargo test --test source_browser -- --ignored"]
async fn capture_budgets_and_inaccessible_css_are_explicit_incomplete_evidence() {
    use axum::{Router, response::Html, routing::get};
    let remote = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_url = format!("http://{}/style.css", remote.local_addr().unwrap());
    let remote_server = tokio::spawn(async move {
        axum::serve(
            remote,
            Router::new().route(
                "/style.css",
                get(|| async { ([("content-type", "text/css")], CSS) }),
            ),
        )
        .await
        .unwrap()
    });
    let html = format!(
        r#"<!doctype html><title>Blocked CSS</title><link rel="stylesheet" href="{remote_url}"><div id="shadow"></div><script>document.querySelector('#shadow').attachShadow({{mode:'open'}}).innerHTML='<p>inside</p>'</script>"#
    );
    let app = Router::new()
        .route("/", get(|| async { Html(document()) }))
        .route(
            "/style.css",
            get(|| async { ([("content-type", "text/css")], CSS) }),
        )
        .route(
            "/blocked",
            get(move || {
                let html = html.clone();
                async move { Html(html) }
            }),
        )
        .route("/missing",get(||async{Html("<!doctype html><link rel=stylesheet href=/missing.css><p>Missing CSS</p><input type=password value=fixture-secret>")}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    tokio::task::spawn_blocking(move||{
        let browser=launch();let tab=browser.new_tab().unwrap();tab.navigate_to(&url).unwrap().wait_until_navigated().unwrap();
        let base=json!({"bytes":evidence::MAX_BYTES,"nodes":evidence::MAX_NODES,"depth":256,"styles":4096,"rules":20000,"value_bytes":1048576});
        for (key,value,expected) in [("nodes",2,"node_limit"),("depth",0,"depth_limit"),("styles",0,"style_limit"),("rules",0,"rule_limit"),("value_bytes",512,"value_limit"),("bytes",70000,"byte_limit")] {
            let mut limits=base.clone();limits[key]=json!(value);
            let raw=tab.evaluate(&format!("({})({limits})",evidence::CAPTURE_SCRIPT),true).unwrap().value.unwrap();
            let capture:evidence::Capture=serde_json::from_str(raw.as_str().unwrap()).unwrap();capture.validate().unwrap();
            assert!(!capture.complete,"{key}");assert!(capture.issues.iter().any(|i|i.code==expected),"{key}: {:?}",capture.issues);
            assert!(capture.require_complete().is_err());
        }
        tab.navigate_to(&format!("{url}blocked")).unwrap().wait_until_navigated().unwrap();
        let root=tempfile::tempdir().unwrap();let paths=evidence::capture_page(&tab,root.path(),0,&evidence::CaptureOptions{viewports:vec!["375x812".parse().unwrap()]},&url::Url::parse(&url).unwrap()).unwrap();
        let capture=evidence::load(root.path(),std::path::Path::new(&paths[0])).unwrap();
        assert!(!capture.complete);assert!(capture.issues.iter().any(|i|i.code=="stylesheet_unavailable"));
        assert!(capture.issues.iter().any(|i|i.code=="shadow_document"));
        assert_eq!(evidence::inspect(root.path()).unwrap().complete,0);
        let before=std::fs::read(root.path().join(&paths[0])).unwrap();
        assert!(evidence::capture_page(&tab,root.path(),0,&evidence::CaptureOptions{viewports:vec!["375x812".parse().unwrap()]},&url::Url::parse(&url).unwrap()).is_err());
        assert_eq!(before,std::fs::read(root.path().join(&paths[0])).unwrap());
        let viewport=tab.evaluate("innerWidth",false).unwrap().value.unwrap();assert_eq!(viewport,json!(1440));
        tab.navigate_to(&format!("{url}missing")).unwrap().wait_until_navigated().unwrap();
        let raw=tab.evaluate(&format!("({})({base})",evidence::CAPTURE_SCRIPT),true).unwrap().value.unwrap();
        assert!(!raw.as_str().unwrap().contains("fixture-secret"));
        let capture:evidence::Capture=serde_json::from_str(raw.as_str().unwrap()).unwrap();capture.validate().unwrap();
        assert!(capture.stylesheets.iter().all(|sheet|sheet.rules.is_empty()));
        assert!(capture.issues.iter().any(|i|i.code=="resource_unavailable"));
        assert!(capture.resources.iter().any(|r|r.url.ends_with("/missing.css")&&r.response_status==Some(404)));
        assert!(capture.issues.iter().any(|i|i.code=="sensitive_value"));
    }).await.unwrap();
    server.abort();
    remote_server.abort();
}
