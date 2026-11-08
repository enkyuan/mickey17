use crate::{
    cache::{AssetCache, AssetResponse},
    util,
};
use anyhow::{Context, Result, ensure};
use regex::Regex;
use reqwest::blocking::Client;
use std::{
    collections::{BTreeSet, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    sync::LazyLock,
};
use url::Url;
pub const MAX_BODY: u64 = 64 * 1024 * 1024;
const MAX_ASSETS: usize = 100_000;

struct AssetQueue {
    todo: VecDeque<Url>,
    discovered: BTreeSet<Url>,
    limit: usize,
}
impl AssetQueue {
    fn new(limit: usize) -> Self {
        Self {
            todo: VecDeque::new(),
            discovered: BTreeSet::new(),
            limit,
        }
    }
    fn schedule(&mut self, urls: impl IntoIterator<Item = Url>) -> Result<()> {
        for u in urls {
            if self.discovered.contains(&u) {
                continue;
            }
            ensure!(
                self.discovered.len() < self.limit,
                "asset discovery limit exceeded"
            );
            self.discovered.insert(u.clone());
            self.todo.push_back(u);
        }
        Ok(())
    }
}
static REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(src|poster|data-src|href|srcset|imagesrcset|data-srcset)=["']([^"']+)["']|url\(\s*["']?([^"')]+?)["']?\s*\)|["'`](?:\./)?(/?(?:[\w-]+/)[^"'`\s?]+?\.(?:js|mjs|css|woff2?|ttf|otf|png|jpe?g|webp|avif|svg|gif|mp4|webm|json|wasm|riv|bin|glb|gltf)(?:\?[^"'`\s]*)?)["'`]"#).expect("valid regex")
});
static ASSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\.(css|js|mjs|ico|png|jpe?g|gif|svg|webp|avif|woff2?|ttf|otf|json|webmanifest|xml|pdf|mp4|webm)$").expect("valid regex")
});
pub fn same_site(a: &Url, b: &Url) -> bool {
    matches!(a.scheme(), "http" | "https")
        && matches!(b.scheme(), "http" | "https")
        && a.host_str().is_some()
        && a.host_str().map(|s| s.trim_start_matches("www."))
            == b.host_str().map(|s| s.trim_start_matches("www."))
        && (a.port_or_known_default() == b.port_or_known_default()
            || (a.port().is_none() && b.port().is_none()))
}
pub fn refs(text: &str, base: &Url) -> BTreeSet<Url> {
    let decoded = html_escape::decode_html_entities(text);
    let mut out = BTreeSet::new();
    for c in REF.captures_iter(&decoded) {
        let kind = c.get(1).map_or("", |m| m.as_str());
        let val = c
            .get(2)
            .or(c.get(3))
            .or(c.get(4))
            .map_or("", |m| m.as_str())
            .trim();
        if val.starts_with("data:") {
            continue;
        }
        for part in if kind.to_lowercase().contains("srcset") {
            val.split(',').collect::<Vec<_>>()
        } else {
            vec![val]
        } {
            let raw = part.split_whitespace().next().unwrap_or("");
            if raw.contains([
                '{', '}', '<', '>', '`', '$', '\\', '^', '|', '(', ')', '*', '#',
            ]) || raw.contains("node_modules/")
            {
                continue;
            }
            let candidate = if c.get(4).is_some() {
                format!("/{}", raw.trim_start_matches('/'))
            } else {
                raw.to_string()
            };
            if let Ok(mut u) = base.join(&candidate) {
                if kind.eq_ignore_ascii_case("href") && !ASSET.is_match(u.path()) {
                    continue;
                }
                if u.path() == "/_next/image" {
                    let src = u
                        .query_pairs()
                        .find(|(k, _)| k == "url")
                        .map(|(_, s)| s.into_owned());
                    let Some(src) = src else { continue };
                    let Ok(original) = base.join(&src) else {
                        continue;
                    };
                    u = original;
                }
                u.set_fragment(None);
                if matches!(u.scheme(), "http" | "https") && same_site(&u, base) {
                    out.insert(u);
                    if out.len() > MAX_ASSETS {
                        return out;
                    }
                }
            }
        }
    }
    out
}
pub fn output_path(root: &Path, u: &Url, page: bool) -> Result<PathBuf> {
    let mut p = util::contained(root, u.path())?;
    if (page && !matches!(p.extension().and_then(|s| s.to_str()), Some("html" | "htm")))
        || u.path().ends_with('/')
    {
        p = p.join("index.html")
    }
    Ok(p)
}
pub fn fetch(client: &Client, u: &Url, rsc: bool) -> Result<(Vec<u8>, String)> {
    let mut request = client.get(u.clone());
    if rsc {
        request = request.header("RSC", "1")
    }
    let response = request.send()?.error_for_status()?;
    ensure!(
        response.status().is_success(),
        "unexpected HTTP status {}",
        response.status()
    );
    let mime = response
        .headers()
        .get("content-type")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("")
        .to_string();
    ensure!(
        response.content_length().unwrap_or(0) <= MAX_BODY,
        "response exceeds 64 MiB"
    );
    let mut bytes = Vec::new();
    response.take(MAX_BODY + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_BODY, "response exceeds 64 MiB");
    Ok((bytes, mime))
}
#[derive(Default, serde::Serialize)]
pub struct MirrorReport {
    pub saved_pages: usize,
    pub assets_fetched: usize,
    pub cache_revalidations: usize,
    pub failures: Vec<serde_json::Value>,
}

pub struct DownloadOptions {
    pub jobs: usize,
    pub cache_dir: Option<PathBuf>,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            jobs: 4,
            cache_dir: None,
        }
    }
}
pub fn mirror(
    client: &Client,
    pages: &[Url],
    assets: BTreeSet<Url>,
    root: &Path,
) -> Result<MirrorReport> {
    mirror_with_options(client, pages, assets, root, &DownloadOptions::default())
}

pub fn mirror_with_options(
    client: &Client,
    pages: &[Url],
    assets: BTreeSet<Url>,
    root: &Path,
    options: &DownloadOptions,
) -> Result<MirrorReport> {
    ensure!(
        (1..=16).contains(&options.jobs),
        "jobs must be between 1 and 16"
    );
    std::fs::create_dir_all(root)?;
    ensure!(
        !root.symlink_metadata()?.file_type().is_symlink(),
        "output root is a symlink"
    );
    let cache = options
        .cache_dir
        .clone()
        .map(AssetCache::open)
        .transpose()?;
    if let Some(cache) = &cache {
        let output = root.canonicalize()?;
        ensure!(
            !cache.root().starts_with(&output) && !output.starts_with(cache.root()),
            "cache and mirrored site directories must not overlap"
        );
    }
    let mut report = MirrorReport::default();
    let mut queue = AssetQueue::new(MAX_ASSETS);
    queue.schedule(assets)?;
    let mut done = BTreeSet::new();
    let page_paths: BTreeSet<_> = pages
        .iter()
        .map(|page| page.path().trim_end_matches('/'))
        .collect();
    let base = pages.first().context("no pages to mirror")?;
    for u in pages {
        let result = (|| -> Result<()> {
            let (bytes, _) = fetch(client, u, false)?;
            let path = output_path(root, u, true)?;
            util::write(root, &path, &bytes)?;
            report.saved_pages += 1;
            let text = String::from_utf8_lossy(&bytes);
            queue.schedule(refs(&text, u))?;
            if text.contains("self.__next_f.push(")
                && path.file_name().is_some_and(|s| s == "index.html")
                && let Ok((rsc, mime)) = fetch(client, u, true)
                && mime.contains("text/x-component")
            {
                util::write(root, &path.with_file_name("index.rsc"), &rsc)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            report
                .failures
                .push(serde_json::json!({"url":u,"error":e.to_string()}))
        }
    }
    for path in [
        "/sw.js",
        "/service-worker.js",
        "/manifest.json",
        "/site.webmanifest",
        "/favicon.ico",
        "/apple-touch-icon.png",
        "/robots.txt",
    ] {
        let u = base.join(path)?;
        if let Ok((bytes, mime)) = fetch(client, &u, false)
            && !mime.contains("text/html")
        {
            let p = output_path(root, &u, false)?;
            if !p.exists() {
                util::write(root, &p, &bytes)?;
                queue.schedule(refs(&String::from_utf8_lossy(&bytes), &u))?;
                done.insert(u);
                report.assets_fetched += 1;
            }
        }
    }
    while !queue.todo.is_empty() {
        let mut batch = Vec::new();
        while batch.len() < options.jobs {
            let Some(u) = queue.todo.pop_front() else {
                break;
            };
            if done.contains(&u)
                || page_paths.contains(u.path().trim_end_matches('/'))
                || u.path().contains("/cdn-cgi/")
            {
                continue;
            }
            done.insert(u.clone());
            batch.push(u);
        }
        let cache_ref = cache.as_ref();
        let responses: Vec<_> = std::thread::scope(|scope| {
            let workers: Vec<_> = batch
                .iter()
                .map(|u| {
                    scope.spawn(move || {
                        if let Some(cache) = cache_ref {
                            cache.fetch(client, u)
                        } else {
                            fetch(client, u, false).map(|(bytes, content_type)| AssetResponse {
                                bytes,
                                content_type,
                                reused: false,
                            })
                        }
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .map_err(|_| anyhow::anyhow!("asset worker panicked"))
                        .and_then(|response| response)
                })
                .collect()
        });
        // Apply in discovery order even when downloads finish in a different order.
        // This preserves collision handling and keeps all filesystem writes sequential.
        for (u, response) in batch.into_iter().zip(responses) {
            let result = (|| -> Result<()> {
                let AssetResponse {
                    bytes,
                    content_type: mime,
                    reused,
                } = response?;
                report.cache_revalidations += usize::from(reused);
                if mime.contains("text/html") && Path::new(u.path()).extension().is_none() {
                    return Ok(());
                }
                let p = output_path(root, &u, false)?;
                ensure!(!p.is_dir(), "asset path collides with directory");
                // Query variants cannot silently replace another response at the same path.
                if p.exists() {
                    ensure!(
                        std::fs::read(&p)? == bytes,
                        "query/path collision: {}",
                        p.display()
                    );
                    return Ok(());
                }
                util::write(root, &p, &bytes)?;
                report.assets_fetched += 1;
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    queue.schedule(refs(text, &u))?;
                }
                Ok(())
            })();
            if let Err(e) = result {
                report
                    .failures
                    .push(serde_json::json!({"url":u,"error":e.to_string()}))
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_deduplicates_at_admission_and_never_exceeds_cap() {
        let mut q = AssetQueue::new(2);
        let a = Url::parse("https://example.com/a.js").unwrap();
        let b = Url::parse("https://example.com/b.js").unwrap();
        q.schedule([a.clone(), b.clone(), a.clone(), b]).unwrap();
        assert_eq!(q.todo.len(), 2);
        q.todo.pop_front();
        q.schedule([a]).unwrap();
        assert_eq!(q.todo.len(), 1);
        assert!(
            q.schedule([Url::parse("https://example.com/c.js").unwrap()])
                .is_err()
        );
        assert_eq!(q.discovered.len(), 2);
        assert_eq!(q.todo.len(), 1);
    }
}
