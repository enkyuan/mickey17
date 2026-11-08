//! End-to-end source export on owned fixtures. Installs the emitted pinned lockfile,
//! typechecks/builds, starts production SSR, then checks Chromium and pixel parity.
#![cfg(any(target_os = "macos", target_os = "linux"))]
use headless_chrome::{
    Browser, LaunchOptions,
    protocol::cdp::{Emulation, Page, Runtime},
};
use mochi::{crawl, evidence, export, project};
use serde_json::{Value, json};
use std::{
    io::Cursor,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const ONE: &str = "/catalog?kind=one&label=a%20b&tag=1&tag=2";
const TWO: &str = "/catalog?kind=two&tag=2&tag=1&label=a+b";
const CSS: &str = r#"@font-face{font-family:Fixture;src:url('/fixture.woff2') format('woff2');font-display:swap}
:root{--duration:.01s;--brand:#203448;--panel:#eef2f6}body{margin:0;background:white;color:var(--brand);font:16px Arial,sans-serif}header,footer{padding:16px;background:var(--panel)}nav{display:flex;gap:16px}main{padding:24px;max-width:900px;margin:auto}@keyframes fixture-enter{from{opacity:0}to{opacity:1}}h1{animation:fixture-enter var(--duration) both;font-family:Fixture,sans-serif;font-size:24px}section{display:grid;grid-template-columns:1fr 1fr;gap:16px}.card{padding:12px;border:1px solid #ccc;border-radius:8px;background:var(--panel);font-size:20px}.card::before{content:'Offer';display:block;color:#555}.card img{width:64px;height:64px}.action{font:inherit;padding:8px 12px}label{display:block;margin:12px 0}input{font:inherit}svg{width:20px;height:20px}.unused::after{content:"</style><script>window.cssInjected=true</script>"}@media(max-width:500px){section{grid-template-columns:1fr}.card{font-size:12px}main{padding:12px}}"#;
fn html(detail: bool, two: bool) -> String {
    let one = ONE.replace('&', "&amp;");
    let two_href = TWO.replace('&', "&amp;");
    let label = if detail {
        "Detail"
    } else if two {
        "Catalog two"
    } else {
        "Catalog one"
    };
    let two = two_href;
    let page = format!(
        r#"<!doctype html><html lang="en"><head><base href="/"><title>{label}</title><meta name="description" content="Source export fixture"><link rel="stylesheet" href="/style.css"></head><body><header><nav><a id="one" href="{one}#cards">One</a><a id="two" href="{two}#cards">Two</a><a id="detail" href="/products/item">Detail</a></nav></header><main><h1>{label}</h1><p id="waapi" style="opacity:0">Observed script animation</p><section id="cards"><article class="card"><a href="/products/item">{label} first</a><img src="/image.png" alt="First"><p>Literal &lt;script&gt;window.injected=true&lt;/script&gt;</p></article><article class="card"><a href="/products/item">{label} second</a><img src="/image.png" alt="Second"><p>Different content</p></article></section><label>Observed value<input id="live" value="original"></label><label>Observed checkbox<input id="check" type="checkbox" checked></label><label>Choice<select id="choice"><option value="a">A</option><option value="b">B</option></select></label><button id="action" class="action" type="button">Action</button><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><title>Fixture icon</title><path d="M2 2h16v16H2z" fill="currentColor"/><line x1="0" y1="0" x2="20" y2="20" stroke="currentColor"/><text x="1" y="10" font-size="4">SVG</text></svg></main><footer>Shared footer</footer><script>document.querySelector('#live').value='live fixture';document.querySelector('#check').checked=false;document.querySelector('#choice').value='b';window.originalBundleRan=true;</script></body></html>"#
    );
    page.replace("<section id=\"cards\">",r##"<div id="priority" style="color:rgb(1,2,3)!important">Priority</div><canvas id="drawing" width="180" height="40" style="display:block"></canvas><canvas id="gpu" width="180" height="30" style="display:block"></canvas><demo-shadow id="shadow" style="display:block">Light</demo-shadow><iframe id="frame" title="Captured frame" style="width:180px;height:45px;border:0;display:block" srcdoc="<style>body{margin:0;background:rgb(220,230,240);font:16px Arial}p{margin:0}</style><p style='opacity:0'>Frame source</p><script>document.querySelector('p').animate([{opacity:0},{opacity:1}],{duration:10,fill:'forwards'})</script>"></iframe><section id="cards">"##)
        .replace("window.originalBundleRan=true;",r##"window.originalBundleRan=true;document.querySelector('#waapi').animate([{opacity:0},{opacity:1}],{duration:10,fill:'forwards'});const ctx=document.querySelector('#drawing').getContext('2d');ctx.fillStyle='#c03020';ctx.fillRect(0,0,180,40);ctx.fillStyle='#203040';ctx.fillRect(20,5,70,20);const gl=document.querySelector('#gpu').getContext('webgl');if(gl){gl.clearColor(.2,.4,.6,1);gl.clear(gl.COLOR_BUFFER_BIT)}const shadow=document.querySelector('#shadow').attachShadow({mode:'open'});shadow.innerHTML='<style>span{color:rgb(12,34,56)}</style><span>Shadow source <slot></slot></span>';"##)
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn run(root: &Path, args: &[&str]) {
    let result = Command::new("bun")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "bun {args:?}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
fn evaluate(tab: &headless_chrome::Tab, script: &str) -> Value {
    let params: Runtime::Evaluate = serde_json::from_value(
        json!({"expression":script,"awaitPromise":true,"returnByValue":true}),
    )
    .unwrap();
    let result = tab.call_method(params).unwrap();
    assert!(
        result.exception_details.is_none(),
        "JavaScript failed: {:?}",
        result.exception_details
    );
    result.result.value.unwrap_or(Value::Null)
}
fn pixels(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    let samples = info.color_type.samples();
    let rgb = buffer[..info.buffer_size()]
        .chunks_exact(samples)
        .flat_map(|p| p[..3].iter().copied())
        .collect();
    (info.width, info.height, rgb)
}
fn compare(reference: &[u8], actual: &[u8]) -> f64 {
    let (w, h, a) = pixels(reference);
    let (w2, h2, b) = pixels(actual);
    assert_eq!((w, h), (w2, h2));
    // Preregistered: at most 0.25% of pixels may differ by more than 2/channel.
    // Do not widen this tolerance after observing a failing export.
    let bad = a
        .as_chunks::<3>()
        .0
        .iter()
        .zip(b.as_chunks::<3>().0)
        .filter(|(a, b)| a.iter().zip(*b).any(|(&x, &y)| x.abs_diff(y) > 2))
        .count();
    bad as f64 / f64::from(w * h)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Bun 1.4.4, package registry access and Chrome; cargo test --test export_browser -- --ignored"]
async fn generated_project_installs_builds_and_matches_owned_reference_states() {
    // FAKE FOR TESTS ONLY: qualify the annotation/export boundary, not Jev quality.
    struct ControlledUnknown;
    impl mochi::interpret::Provider for ControlledUnknown {
        fn origin(&self) -> mochi::interpret::Origin {
            mochi::interpret::Origin::ControlledFixture
        }
        fn ask(
            &self,
            request: &mochi::interpret::Request,
            _: Duration,
        ) -> anyhow::Result<mochi::interpret::Reply> {
            use mochi::interpret::{Answer, Question, Reply, Response, Usage};
            let answers = request
                .questions
                .iter()
                .map(|(id, question)| {
                    let answer = match question {
                        Question::Noul { .. } => Answer::Noul { noul: 0.1 },
                        Question::Choice { criteria, .. } => Answer::Choice {
                            choice: "unknown".into(),
                            confidence: 1.0,
                            probabilities: criteria
                                .keys()
                                .map(|key| (key.clone(), if key == "unknown" { 1.0 } else { 0.0 }))
                                .collect(),
                        },
                    };
                    (id.clone(), answer)
                })
                .collect();
            Ok(Reply::Response(Response {
                model: request.model.clone(),
                answers,
                usage: Usage {
                    input_tokens: 123,
                    output_tokens: 20,
                },
            }))
        }
    }
    use axum::{Router, response::Html, routing::get};
    let app = Router::new()
        .route(
            "/catalog",
            get(|uri: axum::http::Uri| async move {
                Html(html(
                    false,
                    uri.query().is_some_and(|q| q.starts_with("kind=two")),
                ))
            }),
        )
        .route("/products/item", get(|| async { Html(html(true, false)) }))
        .route(
            "/style.css",
            get(|| async { ([("content-type", "text/css")], CSS) }),
        )
        .route(
            "/fixture.woff2",
            get(|| async {
                (
                    [("content-type", "font/woff2")],
                    include_bytes!("fixtures/fixture-font.woff2").as_slice(),
                )
            }),
        )
        .route(
            "/image.png",
            get(|| async {
                (
                    [("content-type", "image/png")],
                    include_bytes!("fixtures/source-reference.png").as_slice(),
                )
            }),
        )
        .route(
            "/favicon.ico",
            get(|| async {
                (
                    [("content-type", "image/png")],
                    include_bytes!("fixtures/source-reference.png").as_slice(),
                )
            }),
        )
        .route("/robots.txt", get(|| async { "User-agent: *\nAllow: /\n" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("capture");
    let generated = std::env::var_os("MOCHI_EXPORT_ARTIFACT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| work.path().join("generated"));
    let artifact = generated.clone();
    tokio::task::spawn_blocking(move||{
        let report=crawl::clone_with_evidence(&format!("{origin}{ONE}"),&source,&crawl::Options{max_pages:3,extract:false,keep_trackers:false,ignore_robots:false},&crawl::PipelineOptions::default(),&evidence::CaptureOptions{viewports:vec!["375x812".parse().unwrap(),"1440x900".parse().unwrap()]}).unwrap();
        assert_eq!(report.saved_pages,2);assert_eq!(report.failures.len(),1,"{:?}",report.failures);assert!(format!("{:?}",report.failures).contains("query/path collision"));
        project::build(&source).unwrap();let model=evidence::inspect(&source).unwrap();
        mochi::interpret::save_plan(&source,mochi::interpret::PlanConfig::default()).unwrap();
        mochi::interpret::with_provider(&source,&ControlledUnknown,mochi::interpret::Limits{max_requests:100,max_input_tokens:6_553_600,max_cost_microusd:1_000_000,input_price_microusd_per_million:42_000,deadline_seconds:60,max_retries:0}).unwrap();
        let report=export::tanstack_start_with_interpretation(&source,&generated).unwrap();assert_eq!(report.routes,3);assert!(report.components>=3);assert!(report.layouts>=1);
        let annotations=report.interpretation.as_ref().unwrap();assert!(!annotations.auto_applied);assert!(!annotations.components.is_empty());assert_eq!(annotations.provider_origins,vec![mochi::interpret::Origin::ControlledFixture]);
        run(&generated,&["install","--frozen-lockfile","--ignore-scripts"]);run(&generated,&["run","build"]);run(&generated,&["run","check"]);
        let socket=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let port=socket.local_addr().unwrap().port();drop(socket);
        let log=std::fs::File::create(generated.join("production-test.log")).unwrap();
        let process=Command::new("bun").arg(".output/server/index.mjs").env("PORT",port.to_string()).env("HOST","127.0.0.1").current_dir(&generated).stdout(Stdio::from(log.try_clone().unwrap())).stderr(Stdio::from(log)).spawn().unwrap();let mut process=Process(process);
        let local=format!("http://127.0.0.1:{port}");let client=reqwest::blocking::Client::new();let deadline=Instant::now()+Duration::from_secs(25);
        loop {if client.get(format!("{local}{ONE}")).send().is_ok_and(|r|r.status().is_success()){break;}assert!(Instant::now()<deadline,"production server did not become ready: {:?}",process.0.try_wait());std::thread::sleep(Duration::from_millis(100));}
        for path in [ONE,TWO,"/products/item"] {assert_eq!(client.get(format!("{local}{path}")).send().unwrap().status(),200);}
        for path in ["/catalog?kind=uncaptured","/not-captured"] {assert_eq!(client.get(format!("{local}{path}")).send().unwrap().status(),404);}
        let mut options=LaunchOptions::default_builder();options.window_size(Some((1440,900))).args(vec![std::ffi::OsStr::new("--hide-scrollbars")]);if let Some(path)=std::env::var_os("MOCHI_CHROME"){options.path(Some(path.into()));}
        let browser=Browser::new(options.build().unwrap()).unwrap();let tab=browser.new_tab().unwrap();
        tab.call_method(Page::AddScriptToEvaluateOnNewDocument{source:"window.fixtureErrors=[];const old=console.error;console.error=(...args)=>{window.fixtureErrors.push(args.map(String).join(' '));old(...args)};window.addEventListener('error',e=>{if(e.message)window.fixtureErrors.push(e.message)});window.addEventListener('unhandledrejection',e=>window.fixtureErrors.push(String(e.reason)))".into(),world_name:None,include_command_line_api:None,run_immediately:None}).unwrap();
        let mut comparisons=Vec::new();
        for entry in &model.captures {
            let metrics:Emulation::SetDeviceMetricsOverride=serde_json::from_value(json!({"width":entry.viewport.width,"height":entry.viewport.height,"deviceScaleFactor":1,"mobile":false})).unwrap();tab.call_method(metrics).unwrap();
            let original=url::Url::parse(&entry.url).unwrap();let query=original.query().map(|q|format!("?{q}")).unwrap_or_default();
            tab.navigate_to(&format!("{local}{}{query}",original.path())).unwrap().wait_until_navigated().unwrap();
            evaluate(&tab,"document.fonts.ready.then(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>setTimeout(r,400)))))");
            let state=evaluate(&tab,"({title:document.title,value:document.querySelector('#live').value,checked:document.querySelector('#check').checked,choice:document.querySelector('#choice').value,original:window.originalBundleRan===true,injected:window.injected===true,cssInjected:window.cssInjected===true,font:[...document.fonts].some(f=>f.family.includes('Fixture')&&f.status==='loaded'),size:getComputedStyle(document.querySelector('.card')).fontSize,pseudo:getComputedStyle(document.querySelector('.card'),'::before').content,errors:window.fixtureErrors,resources:performance.getEntriesByType('resource').filter(r=>r.name.includes('woff2')||r.name.includes('png')).map(r=>r.name)})");
            assert_eq!(state["value"],"live fixture");assert_eq!(state["checked"],false);assert_eq!(state["choice"],"b");assert_eq!(state["original"],false);assert_eq!(state["injected"],false);assert_eq!(state["cssInjected"],false);assert_eq!(state["font"],true);assert_eq!(state["pseudo"],"\"Offer\"");assert_eq!(evaluate(&tab,"getComputedStyle(document.querySelector('h1')).opacity"),"1");
            assert_eq!(state["size"],if entry.viewport.width==375{"12px"}else{"20px"});assert_eq!(state["errors"],json!([]),"{state}");
            assert!(state["resources"].as_array().unwrap().iter().any(|u|u.as_str().unwrap().ends_with(".woff2")));assert!(state["resources"].as_array().unwrap().iter().any(|u|u.as_str().unwrap().ends_with(".png")));assert!(state["resources"].as_array().unwrap().iter().all(|u|u.as_str().unwrap().starts_with(&local)),"fonts/images must not depend on the original host: {state}");
            assert_eq!(evaluate(&tab,"document.querySelector('#shadow').shadowRoot.textContent.includes('Shadow source')"),true);
            assert_eq!(evaluate(&tab,"getComputedStyle(document.querySelector('#priority')).color"),"rgb(1, 2, 3)");
            assert_eq!(evaluate(&tab,"getComputedStyle(document.querySelector('#waapi')).opacity"),"1","observed script-created animation state must survive");
            assert_eq!(evaluate(&tab,"document.querySelectorAll('[data-mochi-surface]').length"),2);
            assert_eq!(evaluate(&tab,"Array.from(document.querySelector('#drawing').getContext('2d').getImageData(0,0,1,1).data)"),json!([192,48,32,255]),"generated canvas must actually paint its frame");
            assert_eq!(evaluate(&tab,"Array.from(document.querySelector('#gpu').getContext('2d').getImageData(0,0,1,1).data)"),json!([51,102,153,255]));
            assert_eq!(evaluate(&tab,"document.querySelector('#frame').getAttribute('sandbox')"),"");
            let capture=evidence::load(&source,Path::new(&entry.snapshot)).unwrap();assert_eq!(state["title"],capture.metadata.title);
            for (&node,surface) in &capture.surfaces {
                let Some(image)=&surface.image else{continue};
                if let evidence::Content::Element{attributes,..}=&capture.nodes[node].content {
                    let id=attributes.iter().find(|a|a.name=="id").map(|a|a.value.as_str());
                    let (_,_,rgb)=pixels(&std::fs::read(source.join(&image.path)).unwrap());
                    if id==Some("drawing") {assert!(rgb.as_chunks::<3>().0.iter().all(|p|*p==[192,48,32]||*p==[32,48,64]),"canvas snapshot must contain its own pixels only");}
                    if id==Some("gpu") {assert!(rgb.as_chunks::<3>().0.iter().all(|p|*p==[51,102,153]),"WebGL pixels must actually render");}
                }
            }
            let screenshot=tab.capture_screenshot(Page::CaptureScreenshotFormatOption::Png,None,None,true).unwrap();
            let reference=std::fs::read(source.join(&capture.screenshot.unwrap().path)).unwrap();let difference=compare(&reference,&screenshot);
            std::fs::write(generated.join(format!("reference-{}-{}.png",entry.viewport.width,&mochi::cache::fingerprint(entry.url.as_bytes())[..8])),reference).unwrap();
            std::fs::write(generated.join(format!("actual-{}-{}.png",entry.viewport.width,&mochi::cache::fingerprint(entry.url.as_bytes())[..8])),screenshot).unwrap();
            assert!(difference<=0.0025,"{} at {}px: pixel mismatch {:.3}% exceeds preregistered 0.25%",entry.url,entry.viewport.width,difference*100.);
            comparisons.push(json!({"url":entry.url,"width":entry.viewport.width,"pixel_mismatch":difference}));
        }
        tab.navigate_to(&format!("{local}{ONE}")).unwrap().wait_until_navigated().unwrap();evaluate(&tab,"new Promise(r=>setTimeout(r,400))");
        evaluate(&tab,"window.navigationMarker=123;document.querySelector('#two').click()");
        evaluate(&tab,"new Promise(r=>setTimeout(r,700))");
        assert_eq!(evaluate(&tab,"({marker:window.navigationMarker,title:document.title,query:location.search,hash:location.hash})"),json!({"marker":123,"title":"Catalog two","query":"?kind=two&tag=2&tag=1&label=a+b","hash":"#cards"}));
        evaluate(&tab,"window.buttonClicks=0;document.querySelector('#action').addEventListener('click',()=>window.buttonClicks++);document.querySelector('#action').focus()");tab.press_key(" ").unwrap();
        assert_eq!(evaluate(&tab,"window.buttonClicks"),1);assert_eq!(evaluate(&tab,"document.activeElement.id"),"action");
        evaluate(&tab,"document.querySelector('#detail').click()");evaluate(&tab,"new Promise(r=>setTimeout(r,700))");assert_eq!(evaluate(&tab,"document.title"),"Detail");assert_eq!(evaluate(&tab,"window.navigationMarker"),123);
        assert_eq!(evaluate(&tab,"window.fixtureErrors"),json!([]));
        // Show that an emitted component prop is editable and affects a rebuilt app.
        let route_file=walkdir::WalkDir::new(generated.join("src/features/pages")).into_iter().map(|e|e.unwrap()).filter(|e|e.file_type().is_file()).map(|e|e.path().to_owned()).find(|p|std::fs::read_to_string(p).unwrap().contains("Catalog one first")).unwrap();
        let text=std::fs::read_to_string(&route_file).unwrap();std::fs::write(&route_file,text.replace("Catalog one first","Edited component prop")).unwrap();
        std::fs::write(generated.join("src/styles/tokens.css"), ":root { --brand: rgb(90, 40, 120); }").unwrap();
        let added = generated.join("src/routes/[_]notes/release[.]v1/index.tsx");
        std::fs::create_dir_all(added.parent().unwrap()).unwrap();
        std::fs::write(&added, "import {createFileRoute} from '@tanstack/react-router'; export const Route=createFileRoute('/_notes/release.v1/')({component:()=> <main>Added physical route</main>});").unwrap();
        process.0.kill().unwrap();process.0.wait().unwrap();run(&generated,&["run","build"]);run(&generated,&["run","check"]);
        let edited=Command::new("bun").arg(".output/server/index.mjs").env("PORT",port.to_string()).env("HOST","127.0.0.1").current_dir(&generated).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();let _edited=Process(edited);
        let deadline=Instant::now()+Duration::from_secs(20);loop {if let Ok(response)=client.get(format!("{local}{ONE}")).send() && response.text().unwrap().contains("Edited component prop"){break;}assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(100));}
        for (width,height) in [(1440,900),(375,812)] {
            tab.call_method(serde_json::from_value::<Emulation::SetDeviceMetricsOverride>(json!({"width":width,"height":height,"deviceScaleFactor":1,"mobile":false})).unwrap()).unwrap();
            tab.navigate_to(&format!("{local}{ONE}")).unwrap().wait_until_navigated().unwrap();
            evaluate(&tab,"new Promise(r=>setTimeout(r,300))");
            assert_eq!(evaluate(&tab,"getComputedStyle(document.querySelector('h1')).color"), "rgb(90, 40, 120)");
        }
        let added_response = client.get(format!("{local}/_notes/release.v1/")).send().unwrap();
        assert_eq!(added_response.status(),200);
        assert!(added_response.text().unwrap().contains("Added physical route"));
        std::fs::write(generated.join(".mochi/fixture-verification.json"),serde_json::to_vec_pretty(&json!({"status":"passed","pixel_threshold":0.0025,"channel_tolerance":2,"comparisons":comparisons,"ssr_queries_404s_client_navigation_controls_fonts_svg_and_prop_edit":true,"token_edit_desktop_mobile_and_new_physical_route":true})).unwrap()).unwrap();
    }).await.unwrap();
    server.abort();
    println!("Qualified emitted project at {}", artifact.display());
}
