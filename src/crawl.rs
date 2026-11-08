use crate::{edit, mirror, trackers};
use anyhow::{Context, Result, ensure};
use headless_chrome::{Browser, LaunchOptions};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, VecDeque},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use url::Url;

// Library wait_until_navigated waits for networkAlmostIdle; long-polling pages
// need only their own document's DOMContentLoaded event before extraction.
fn navigate_dom(tab: &headless_chrome::Tab, url: &Url) -> Result<()> {
    use headless_chrome::protocol::cdp::{Page, types::Event};
    let frame = tab
        .call_method(Page::GetFrameTree(None))?
        .frame_tree
        .frame
        .id;
    let (sender, receiver) = std::sync::mpsc::sync_channel(32);
    let listener = tab.add_event_listener(Arc::new(move |event: &Event| {
        if let Event::PageLifecycleEvent(event) = event
            && event.params.frame_id == frame
            && event.params.name == "DOMContentLoaded"
        {
            let _ = sender.try_send(event.params.loader_id.clone());
        }
    }))?;
    let result = (|| -> Result<()> {
        let navigation = tab.call_method(Page::Navigate {
            url: url.to_string(),
            referrer: None,
            transition_Type: None,
            frame_id: None,
            referrer_policy: None,
        })?;
        ensure!(
            navigation.error_text.is_none(),
            "navigation failed: {:?}",
            navigation.error_text
        );
        ensure!(
            navigation.is_download != Some(true),
            "navigation returned a download"
        );
        if let Some(loader) = navigation.loader_id {
            let deadline = std::time::Instant::now() + Duration::from_secs(45);
            loop {
                let loaded = receiver
                    .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .context("document readiness deadline exceeded")?;
                if loaded == loader {
                    break;
                }
            }
        }
        Ok(())
    })();
    tab.remove_event_listener(&listener)?;
    result
}

pub struct Options {
    pub max_pages: usize,
    pub extract: bool,
    pub keep_trackers: bool,
    pub ignore_robots: bool,
}
pub fn page_key(mut u: Url, base: &Url) -> Option<Url> {
    if !matches!(u.scheme(), "http" | "https") || !mirror::same_site(&u, base) {
        return None;
    }
    let ext = Path::new(u.path())
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "svg"
            | "webp"
            | "avif"
            | "ico"
            | "pdf"
            | "zip"
            | "mp4"
            | "webm"
            | "mp3"
            | "woff"
            | "woff2"
            | "ttf"
            | "otf"
            | "css"
            | "js"
            | "mjs"
            | "json"
            | "xml"
            | "txt"
            | "rss"
    ) {
        return None;
    }
    let path = u.path().trim_end_matches('/').to_string();
    u.set_path(if path.is_empty() { "/" } else { &path });
    u.set_query(None);
    u.set_fragment(None);
    Some(u)
}
const EXTRACT: &str = r#"JSON.stringify({title:document.title,lang:document.documentElement.lang,canonical:document.querySelector('link[rel=canonical]')?.href||null,meta:Object.fromEntries([...document.querySelectorAll('meta')].map(m=>[m.name||m.getAttribute('property')||m.httpEquiv,m.content])),text:document.body?.innerText||'',headings:[...document.querySelectorAll('h1,h2,h3,h4,h5,h6')].map(h=>({tag:h.tagName,text:h.innerText})),links:[...document.querySelectorAll('a[href]')].map(a=>({href:a.href,text:a.innerText})),images:[...document.images].map(i=>({src:i.currentSrc||i.src,alt:i.alt})),forms:[...document.forms].map(f=>({action:f.action,method:f.method,fields:[...f.elements].map(i=>({tag:i.tagName,type:i.type,name:i.name}))})),jsonld:[...document.querySelectorAll('script[type="application/ld+json"]')].map(s=>s.textContent)})"#;
pub fn clone_site(start: &str, out: &Path, options: &Options) -> Result<mirror::MirrorReport> {
    ensure!(options.max_pages > 0, "max-pages must be positive");
    let base = Url::parse(&if start.contains("://") {
        start.to_string()
    } else {
        format!("https://{start}")
    })?;
    ensure!(
        matches!(base.scheme(), "http" | "https") && base.host_str().is_some(),
        "expected HTTP(S) URL"
    );
    ensure!(
        !out.exists() || (out.is_dir() && out.read_dir()?.next().is_none()),
        "output directory must be new or empty"
    );
    if let Ok(meta) = out.symlink_metadata() {
        ensure!(
            !meta.file_type().is_symlink(),
            "output directory must not be a symlink"
        );
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("mochi/0.2")
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if attempt
                .previous()
                .first()
                .is_some_and(|first| !mirror::same_site(first, attempt.url()))
            {
                attempt.error("cross-site HTTP redirect refused")
            } else {
                attempt.follow()
            }
        }))
        .build()?;
    let robots_url = base.join("/robots.txt")?;
    let robots = match client.get(robots_url).send() {
        Ok(r) if r.status().is_success() => {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut std::io::Read::take(r, mirror::MAX_BODY), &mut bytes)?;
            String::from_utf8_lossy(&bytes).into_owned()
        }
        Ok(r) if r.status().is_client_error() => String::new(),
        result if options.ignore_robots => {
            drop(result);
            String::new()
        }
        _ => anyhow::bail!("robots.txt unavailable; retry or use --ignore-robots"),
    };
    let first = page_key(base.clone(), &base).context("start URL is not a page")?;
    let mut queue = VecDeque::from([first.clone()]);
    let mut seen = BTreeSet::from([first]);
    let loc = Regex::new(r"<loc>\s*(.*?)\s*</loc>")?;
    let mut sitemaps: Vec<String> = robots
        .lines()
        .filter_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("sitemap"))
                .map(|(_, s)| s.trim().into())
        })
        .collect();
    if sitemaps.is_empty() {
        sitemaps.push(base.join("/sitemap.xml")?.to_string())
    }
    for sm in sitemaps.into_iter().take(32) {
        if let Ok(u) = base.join(&sm)
            && let Ok((bytes, _)) = mirror::fetch(&client, &u, false)
        {
            for cap in loc.captures_iter(&String::from_utf8_lossy(&bytes)) {
                if let Ok(u) = base.join(&html_escape::decode_html_entities(&cap[1]))
                    && let Some(u) = page_key(u, &base)
                    && seen.len() < 100_000
                    && seen.insert(u.clone())
                {
                    queue.push_back(u)
                }
            }
        }
    }
    let mut launch = LaunchOptions::default_builder();
    launch
        .window_size(Some((1440, 900)))
        .ignore_certificate_errors(false);
    if let Some(path) = std::env::var_os("MOCHI_CHROME") {
        launch.path(Some(path.into()));
    }
    let browser = Browser::new(launch.build().map_err(|e| anyhow::anyhow!(e))?)
        .context("launch Chromium; install Chrome or set MOCHI_CHROME")?;
    let tab = browser.new_tab()?;
    tab.set_default_timeout(Duration::from_secs(45));
    let assets = Arc::new(Mutex::new(BTreeSet::new()));
    let external = Arc::new(Mutex::new(BTreeSet::new()));
    let (a, e, b) = (assets.clone(), external.clone(), base.clone());
    tab.register_response_handling(
        "capture",
        Box::new(move |event, _| {
            if let Ok(u) = Url::parse(&event.response.url) {
                if mirror::same_site(&u, &b) {
                    if let Ok(mut a) = a.lock()
                        && a.len() < 100_000
                    {
                        a.insert(u);
                    }
                } else if let Ok(mut e) = e.lock() {
                    e.insert(u.host_str().unwrap_or("").to_string());
                }
            }
        }),
    )?;
    let mut pages = Vec::new();
    let mut metadata = Vec::new();
    let mut skipped = Vec::new();
    while let Some(u) = queue.pop_front() {
        if pages.len() >= options.max_pages {
            break;
        }
        if !options.ignore_robots
            && !robotstxt::DefaultMatcher::default().one_agent_allowed_by_robots(
                &robots,
                "mochi",
                u.as_str(),
            )
        {
            skipped.push(json!({"url":u,"reason":"robots.txt"}));
            continue;
        }
        println!("  [{}] {u}", pages.len() + 1);
        let result = (|| -> Result<()> {
            navigate_dom(&tab, &u)?;
            let status = tab
                .evaluate(
                    "performance.getEntriesByType('navigation')[0]?.responseStatus || 0",
                    false,
                )?
                .value
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            ensure!(status < 400, "HTTP {status}");
            ensure!(
                tab.evaluate("document.contentType.includes('html')", false)?
                    .value
                    == Some(json!(true)),
                "not an HTML document"
            );
            tab.evaluate("(async()=>{for(let y=0,n=0;y<(document.body?.scrollHeight||0)&&n<200;y+=600,n++){scrollTo(0,y);await new Promise(r=>setTimeout(r,120))}scrollTo(0,0);await new Promise(r=>setTimeout(r,400))})()",true)?;
            let data = tab
                .evaluate(EXTRACT, false)?
                .value
                .context("missing page data")?;
            let data: Value = serde_json::from_str(data.as_str().context("invalid page data")?)?;
            let final_url = Url::parse(&tab.get_url())?;
            ensure!(
                mirror::same_site(&final_url, &base),
                "page redirected outside site"
            );
            if options.extract {
                let dir = out.join("extract");
                std::fs::create_dir_all(&dir)?;
                let name = format!("page-{:05}", pages.len());
                std::fs::write(
                    dir.join(format!("{name}.json")),
                    serde_json::to_vec_pretty(&data)?,
                )?;
                std::fs::write(
                    dir.join(format!("{name}.txt")),
                    data["text"].as_str().unwrap_or(""),
                )?;
                let dimensions = tab.evaluate("JSON.stringify({width:Math.max(document.documentElement.scrollWidth,1440),height:Math.max(document.documentElement.scrollHeight,900)})", false)?.value.context("missing screenshot dimensions")?;
                let dimensions: Value = serde_json::from_str(
                    dimensions
                        .as_str()
                        .context("invalid screenshot dimensions")?,
                )?;
                let png = tab.capture_screenshot(
                    headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png,
                    None,
                    Some(headless_chrome::protocol::cdp::Page::Viewport {
                        x: 0.0,
                        y: 0.0,
                        width: dimensions["width"]
                            .as_f64()
                            .context("invalid screenshot width")?,
                        height: dimensions["height"]
                            .as_f64()
                            .context("invalid screenshot height")?,
                        scale: 1.0,
                    }),
                    true,
                )?;
                std::fs::write(dir.join(format!("{name}.png")), png)?;
            }
            if let Some(links) = data["links"].as_array() {
                for link in links {
                    if let Some(href) = link["href"].as_str()
                        && let Ok(v) = Url::parse(href)
                        && let Some(v) = page_key(v, &base)
                        && seen.len() < 100_000
                        && seen.insert(v.clone())
                    {
                        queue.push_back(v)
                    }
                }
            }
            metadata
                .push(json!({"url":u,"final_url":final_url,"status":status,"title":data["title"]}));
            pages.push(u.clone());
            Ok(())
        })();
        if let Err(err) = result {
            skipped.push(json!({"url":u,"reason":err.to_string()}))
        }
    }
    tab.deregister_response_handling_all()?;
    drop(tab);
    drop(browser);
    ensure!(!pages.is_empty(), "no pages loaded: {skipped:?}");
    let assets = assets
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock poisoned"))?
        .clone();
    let site = out.join("site");
    let report = mirror::mirror(&client, &pages, assets, &site)?;
    edit::localize(&site, &base)?;
    if !options.keep_trackers {
        trackers::strip(&site)?;
    }
    std::fs::write(
        out.join("manifest.json"),
        serde_json::to_vec_pretty(
            &json!({"source":base,"pages":metadata,"assets_fetched":report.assets_fetched,"failures":report.failures,"skipped":skipped,"external_hosts":*external.lock().map_err(|_|anyhow::anyhow!("capture lock poisoned"))?}),
        )?,
    )?;
    ensure!(
        report.saved_pages > 0,
        "no pages could be saved; see manifest.json"
    );
    Ok(report)
}
