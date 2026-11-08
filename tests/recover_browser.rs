//! Owned scroll/resize/HiDPI fixture. Recovery must preserve executable vector
//! behavior, not approximate the fixture with initial-state screenshots.
#![cfg(any(target_os = "macos", target_os = "linux"))]
use headless_chrome::{
    Browser, LaunchOptions,
    protocol::cdp::{Emulation, Runtime},
};
use mochi::{crawl, evidence, export, project};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const CSS: &str = "body{margin:0;font:16px Arial}main{height:4000px}svg{position:fixed;top:100px;width:100%;height:200px}canvas{position:fixed;bottom:0;left:0;width:100%;height:100px}button{position:fixed;top:40px}";
const DRAW: &str = r#"function paint(){const w=innerWidth,y=scrollY;const svg=document.querySelector('#vector');svg.setAttribute('viewBox',`0 0 ${w} 200`);document.querySelector('#curve').setAttribute('d',`M0 100 Q${w/2} ${100+y/20} ${w} 100`);document.querySelector('#curve').style.strokeDashoffset=String(4000-y);const canvas=document.querySelector('#paint');const d=devicePixelRatio||1;canvas.width=Math.round(w*d);canvas.height=Math.round(100*d);const c=canvas.getContext('2d');c.setTransform(d,0,0,d,0,0);c.fillStyle=y>1000?'rgb(20,40,80)':'rgb(180,60,30)';c.fillRect(0,0,w,100)}"#;
fn factory() -> String {
    format!(
        r#"globalThis.TURBOPACK.push([document.currentScript,7,e=>{{const React=e.i(8),jsx=React.createElement,a=e.i(9),b=e.i(10);{DRAW}function Root(props){{const[count,setCount]=React.useState(0);React.useEffect(()=>{{if(b.value!==2||Object.keys(b).join()!=='value'||a.__esModule!==true||Object.prototype.toString.call(a)!=='[object Module]'||!Object.isSealed(a))throw Error('namespace ABI mismatch');const before=a.value;a.advance();if(a.value!==before+1||b.value!==2)throw Error('live export mismatch');let frame;const update=()=>{{cancelAnimationFrame(frame);frame=requestAnimationFrame(paint)}};addEventListener('scroll',update);addEventListener('resize',update);update();return()=>{{cancelAnimationFrame(frame);removeEventListener('scroll',update);removeEventListener('resize',update)}}}},[]);return jsx('main',null,jsx('h1',null,props.title),jsx('button',{{id:'counter',onClick:()=>setCount(n=>n+1)}},'Count '+count),jsx('svg',{{id:'vector'}},jsx('path',{{id:'curve',stroke:'black',fill:'none',strokeDasharray:'4000'}})),jsx('canvas',{{id:'paint'}}))}}e.s(['default',()=>Root])}},8,(e,t,n)=>{{/* FAKE FOR TESTS ONLY: React ABI marker; the emitted bridge uses installed React. */Symbol.for('react.transitional.element');n.useState=()=>{{}};n.useEffect=()=>{{}};n.createElement=()=>{{}}}},9,10,e=>{{let x=1,y=2;e.s(['value',()=>x,'advance',0,()=>{{x++}}],9);e.s(['value',()=>y],10)}}]);"#
    )
}
fn html() -> String {
    format!(
        r#"<!doctype html><html><head><title>Motion fixture</title><style>{CSS}</style></head><body><main><h1>Motion fixture</h1><button id="counter">Count 0</button><svg id="vector"><path id="curve" stroke="black" fill="none" stroke-dasharray="4000"/></svg><canvas id="paint"></canvas></main><script>self.__next_f=[];self.__next_f.push([0]);globalThis.TURBOPACK=[];</script><script src="/_next/static/chunks/client.js"></script><script>{DRAW};addEventListener('scroll',paint);addEventListener('resize',paint);paint()</script></body></html>"#
    )
}
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn evaluate(tab: &headless_chrome::Tab, source: &str) -> Value {
    let params: Runtime::Evaluate = serde_json::from_value(
        json!({"expression":source,"awaitPromise":true,"returnByValue":true}),
    )
    .unwrap();
    let result = tab.call_method(params).unwrap();
    assert!(
        result.exception_details.is_none(),
        "{:?}",
        result.exception_details
    );
    result.result.value.unwrap_or(Value::Null)
}
fn bun(dir: &std::path::Path, args: &[&str]) {
    let output = Command::new("bun")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Chrome, Node and package registry; run cargo test --test recover_browser -- --ignored"]
async fn recovered_source_preserves_scroll_resize_hidpi_and_react_interactions() {
    use axum::{
        Router,
        http::HeaderMap,
        response::{Html, IntoResponse},
        routing::get,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app=Router::new().route("/",get(|headers:HeaderMap|async move{if headers.contains_key("rsc"){([("content-type","text/x-component")],"1:I[7,[],\"default\"]\n0:[\"$\",\"$L1\",null,{\"title\":\"Motion fixture\"}]\n").into_response()}else{Html(html()).into_response()}}))
        .route("/_next/static/chunks/client.js",get(||async{([("content-type","text/javascript")],factory())}))
        .route("/robots.txt",get(||async{"User-agent: *\nAllow: /\n"}));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    tokio::task::spawn_blocking(move||{
        let root=tempfile::tempdir().unwrap();let source=root.path().join("capture");
        crawl::clone_with_evidence(&base,&source,&crawl::Options{max_pages:1,extract:false,keep_trackers:false,ignore_robots:false},&crawl::PipelineOptions::default(),&evidence::CaptureOptions{viewports:vec!["375x812".parse().unwrap(),"1440x900".parse().unwrap()]}).unwrap();
        project::build(&source).unwrap();let out=root.path().join("project");export::tanstack_start_with_clients(&source,&out,false,&BTreeSet::from(["/".into()])).unwrap();
        bun(&out,&["install","--frozen-lockfile","--ignore-scripts"]);bun(&out,&["run","build"]);bun(&out,&["run","check"]);
        let socket=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let port=socket.local_addr().unwrap().port();drop(socket);
        let mut command=Command::new("bun");command.arg(".output/server/index.mjs").current_dir(&out).env("PORT",port.to_string()).env("HOST","127.0.0.1").stdout(Stdio::null()).stderr(Stdio::null());let _production=Server(command.spawn().unwrap());
        let client=reqwest::blocking::Client::new();let local=format!("http://127.0.0.1:{port}");let deadline=Instant::now()+Duration::from_secs(20);loop{if client.get(&local).send().is_ok_and(|r|r.status().is_success()){break}assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(100));}
        let mut options=LaunchOptions::default_builder();options.args(vec![std::ffi::OsStr::new("--hide-scrollbars")]);if let Some(path)=std::env::var_os("MOCHI_CHROME"){options.path(Some(path.into()));}let browser=Browser::new(options.build().unwrap()).unwrap();let tab=browser.new_tab().unwrap();
        for (width,height,scale)in[(375,812,1),(1024,768,2),(1920,1080,2)]{
            tab.navigate_to("about:blank").unwrap().wait_until_navigated().unwrap();
            let metrics:Emulation::SetDeviceMetricsOverride=serde_json::from_value(json!({"width":width,"height":height,"deviceScaleFactor":scale,"mobile":false})).unwrap();tab.call_method(metrics).unwrap();
            tab.navigate_to(&local).unwrap().wait_until_navigated().unwrap();tab.wait_for_element("#vector").unwrap();evaluate(&tab,"new Promise(r=>setTimeout(r,600))");
            for y in[0,400,1400,400,0]{
                evaluate(&tab,&format!("window.scrollTo(0,{y})"));evaluate(&tab,"new Promise(r=>setTimeout(r,100))");
                let state=evaluate(&tab,"({y:scrollY,width:innerWidth,path:document.querySelector('#curve').getAttribute('d'),offset:document.querySelector('#curve').style.strokeDashoffset,canvas:document.querySelector('#paint').width,pixel:Array.from(document.querySelector('#paint').getContext('2d').getImageData(0,0,1,1).data),snapshots:document.querySelectorAll('[data-mochi-surface]').length})");
                assert_eq!(state["y"],y);assert_eq!(state["width"],width);assert_eq!(state["canvas"],width*scale);assert_eq!(state["snapshots"],0);
                assert_eq!(state["path"],format!("M0 100 Q{} {} {width} 100",f64::from(width)/2.0,100+y/20));assert_eq!(state["offset"],(4000-y).to_string());
                assert_eq!(state["pixel"],if y>1000{json!([20,40,80,255])}else{json!([180,60,30,255])});
            }
            let resized:Emulation::SetDeviceMetricsOverride=serde_json::from_value(json!({"width":width+137,"height":height,"deviceScaleFactor":scale,"mobile":false})).unwrap();tab.call_method(resized).unwrap();evaluate(&tab,"new Promise(r=>setTimeout(r,150))");assert_eq!(evaluate(&tab,"document.querySelector('#paint').width"),(width+137)*scale);assert_eq!(evaluate(&tab,"document.querySelector('#curve').getAttribute('d')"),format!("M0 100 Q{} 100 {} 100",f64::from(width+137)/2.0,width+137));
            tab.wait_for_element("#counter").unwrap().click().unwrap();evaluate(&tab,"new Promise(r=>setTimeout(r,50))");assert_eq!(evaluate(&tab,"document.querySelector('#counter').textContent"),"Count 1");
        }
        let mut models=Vec::new();for file in walkdir::WalkDir::new(out.join("src/compat/recovered")){let file=file.unwrap();if file.path().extension().is_some_and(|x|x=="js"){models.push(std::fs::read_to_string(file.path()).unwrap());}}assert!(models.iter().any(|s|s.contains("function paint()")));
    }).await.unwrap();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Chrome, Node and package registry; run cargo test --test recover_browser -- --ignored"]
async fn native_custom_elements_initialize_once_from_pristine_markup_and_keep_live_canvas() {
    use axum::{Router, response::Html, routing::get};
    const SCRIPT: &str = r#"class Counter extends HTMLElement{connectedCallback(){if(this.childNodes.length)throw Error('Expected pristine server markup');this.innerHTML='<button id="native-count">Count 0</button><canvas id="native-paint" style="width:100%;height:100px"></canvas>';let n=0;this.querySelector('button').onclick=()=>this.querySelector('button').textContent='Count '+(++n);const canvas=this.querySelector('canvas');const paint=()=>{canvas.width=innerWidth*devicePixelRatio;canvas.height=100*devicePixelRatio;const c=canvas.getContext('2d');c.fillStyle=scrollY>1000?'rgb(10,30,90)':'rgb(190,60,20)';c.fillRect(0,0,canvas.width,canvas.height)};addEventListener('scroll',paint);addEventListener('resize',paint);paint()}}customElements.define('live-counter',Counter);"#;
    let html = r##"<!doctype html><html><head><title>Native fixture</title><style>body{margin:0;height:4000px}live-counter{display:block}canvas{position:fixed;bottom:0;left:0}button{position:fixed;top:0}</style></head><body><img id="native-image" src="/native-logo.svg" width="20" height="20"><a id="native-canonical" href="/catalog">Native link</a><svg><path d="M0 0 L10 10" /></svg><live-counter></live-counter><script type=module src=/native.js></script></body></html>"##;
    use base64::Engine;
    let embedded = base64::engine::general_purpose::STANDARD
        .encode(include_bytes!("fixtures/fixture-font.woff2"));
    let html=html.replace("<style>",&format!("<style>@font-face{{font-family:EmbeddedFixture;src:url(data:font/woff2;base64,{embedded})}}body{{font-family:EmbeddedFixture}}"));
    let app = Router::new()
        .route(
            "/catalog",
            get(|| async { axum::response::Redirect::permanent("/catalog/") }),
        )
        .route(
            "/catalog/",
            get(move || {
                let html = html.clone();
                async move { Html(html) }
            }),
        )
        .route(
            "/native.js",
            get(|| async { ([("content-type", "text/javascript")], SCRIPT) }),
        )
        .route("/native-logo.svg",get(||async{([("content-type","image/svg+xml")],r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><rect width="20" height="20" fill="blue"/></svg>"#)}))
        .route("/robots.txt", get(|| async { "User-agent: *\nAllow: /\n" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/catalog/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    tokio::task::spawn_blocking(move||{
        let root=tempfile::tempdir().unwrap();let source=root.path().join("capture");
        crawl::clone_with_evidence(&base,&source,&crawl::Options{max_pages:1,extract:false,keep_trackers:false,ignore_robots:false},&crawl::PipelineOptions::default(),&evidence::CaptureOptions{viewports:vec!["375x812".parse().unwrap()]}).unwrap();
        project::build(&source).unwrap();let out=root.path().join("project");let report=export::tanstack_start_with_clients(&source,&out,false,&BTreeSet::from(["/catalog/".into()])).unwrap();assert_eq!(report.client_recovery.unwrap().routes["/catalog/"].method,"native_custom_elements_esm");
        bun(&out,&["install","--frozen-lockfile","--ignore-scripts"]);bun(&out,&["run","build"]);bun(&out,&["run","check"]);
        let socket=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let port=socket.local_addr().unwrap().port();drop(socket);
        let mut command=Command::new("bun");command.arg(".output/server/index.mjs").current_dir(&out).env("PORT",port.to_string()).env("HOST","127.0.0.1").stdout(Stdio::null()).stderr(Stdio::null());let _production=Server(command.spawn().unwrap());
        let local=format!("http://127.0.0.1:{port}/catalog/");let client=reqwest::blocking::Client::new();let deadline=Instant::now()+Duration::from_secs(20);loop{if client.get(&local).send().is_ok_and(|r|r.status().is_success()){break}assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(100));}
        let mut options=LaunchOptions::default_builder();if let Some(path)=std::env::var_os("MOCHI_CHROME"){options.path(Some(path.into()));}let browser=Browser::new(options.build().unwrap()).unwrap();let tab=browser.new_tab().unwrap();
        let metrics:Emulation::SetDeviceMetricsOverride=serde_json::from_value(json!({"width":1024,"height":768,"deviceScaleFactor":2,"mobile":false})).unwrap();tab.call_method(metrics).unwrap();tab.navigate_to(&local).unwrap().wait_until_navigated().unwrap();tab.wait_for_element("#native-count").unwrap();
        assert_eq!(evaluate(&tab,"document.querySelector('#native-canonical').getAttribute('href')"),"/catalog/");assert_eq!(evaluate(&tab,"document.querySelector('#native-image').naturalWidth"),20);assert_eq!(evaluate(&tab,"new URL(document.querySelector('#native-image').src).origin===location.origin"),true);assert_eq!(evaluate(&tab,"document.querySelectorAll('#native-count').length"),1);assert_eq!(evaluate(&tab,"document.querySelectorAll('[data-mochi-surface]').length"),0);assert_eq!(evaluate(&tab,"document.querySelector('#native-paint').width"),2048);
        tab.wait_for_element("#native-count").unwrap().click().unwrap();assert_eq!(evaluate(&tab,"document.querySelector('#native-count').textContent"),"Count 1");
        evaluate(&tab,"scrollTo(0,1400);new Promise(r=>setTimeout(r,150))");assert_eq!(evaluate(&tab,"Array.from(document.querySelector('#native-paint').getContext('2d').getImageData(0,0,1,1).data)"),json!([10,30,90,255]));
        let metrics:Emulation::SetDeviceMetricsOverride=serde_json::from_value(json!({"width":1161,"height":768,"deviceScaleFactor":2,"mobile":false})).unwrap();tab.call_method(metrics).unwrap();evaluate(&tab,"new Promise(r=>setTimeout(r,150))");assert_eq!(evaluate(&tab,"document.querySelector('#native-paint').width"),2322);assert_eq!(evaluate(&tab,"document.documentElement.dataset.mochiClientError||null"),Value::Null);
    }).await.unwrap();
    server.abort();
}
