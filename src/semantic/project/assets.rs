//! Assets the assembled project serves: images, media and fonts that the emitted markup or the
//! captured stylesheets reference, copied from the capture's mirrored `site/` into `public/` at
//! the path the URL already has, so root-relative references resolve with no further rewriting.
//! Every copy is recorded with its source, size and SHA-256; everything left behind is listed
//! with the reason.
use crate::{mirror, util};
use cssparser::{ParseError, Parser, Token};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use url::Url;

const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;
/// Everything copied is held in memory until the project is written.
const MAX_TOTAL_BYTES: u64 = 768 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct AssetEntry {
    /// Where the project serves it (`/assets/logo.svg`).
    pub public_path: String,
    /// The URL the capture referenced.
    pub url: String,
    /// Path of the mirrored file relative to the capture directory.
    pub source: String,
    pub bytes: u64,
    pub sha256: String,
    /// What referenced it: `route:/pay`, `css:styles.css`.
    pub referenced_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetSkip {
    pub url: String,
    pub reason: String,
    pub referenced_by: String,
}

/// Files to write (project path, bytes), provenance rows, and what was not copied.
pub type Finished = (Vec<(String, Vec<u8>)>, Vec<AssetEntry>, Vec<AssetSkip>);

pub struct Assets {
    site: PathBuf,
    /// The capture directory, for provenance paths.
    root: PathBuf,
    host: Option<String>,
    found: RefCell<BTreeMap<String, AssetEntry>>,
    data: RefCell<BTreeMap<String, Vec<u8>>>,
    total: std::cell::Cell<u64>,
    skipped: RefCell<Vec<AssetSkip>>,
}

fn is_remote_scheme(raw: &str) -> bool {
    let lower = raw.trim_start().to_ascii_lowercase();
    lower.starts_with("data:")
        || lower.starts_with("blob:")
        || lower.starts_with("javascript:")
        || lower.starts_with('#')
}

impl Assets {
    pub fn new(capture_dir: &Path, origin: &Url) -> Assets {
        Assets {
            site: capture_dir.join("site"),
            root: capture_dir.to_path_buf(),
            host: origin.host_str().map(str::to_owned),
            found: RefCell::new(BTreeMap::new()),
            data: RefCell::new(BTreeMap::new()),
            total: std::cell::Cell::new(0),
            skipped: RefCell::new(Vec::new()),
        }
    }

    /// The path the project serves `raw` from, when the capture holds the file; records the use.
    /// `None` leaves the reference as written (external, missing or not a file).
    pub fn resolve(&self, raw: &str, base: &Url, referrer: &str) -> Option<String> {
        let raw = raw.trim();
        if raw.is_empty() || is_remote_scheme(raw) {
            return None;
        }
        let url = base.join(raw).ok()?;
        let skip = |reason: &str| {
            self.skipped.borrow_mut().push(AssetSkip {
                url: url.to_string(),
                reason: reason.to_owned(),
                referenced_by: referrer.to_owned(),
            });
            None
        };
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        if url.host_str() != self.host.as_deref() {
            return skip("served from another host; not part of this site's mirror");
        }
        let mut bare = url.clone();
        bare.set_query(None);
        bare.set_fragment(None);
        let Ok(path) = mirror::output_path(&self.site, &bare, false)
            .and_then(|p| util::checked_path(&self.site, &p))
        else {
            return skip("path escapes the mirror or cannot be mapped");
        };
        let Ok(meta) = std::fs::metadata(&path) else {
            return skip("not present in the capture's mirror");
        };
        if !meta.is_file() {
            return skip("not a file in the mirror");
        }
        if meta.len() > MAX_ASSET_BYTES {
            return skip("larger than 64 MiB");
        }
        if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("html" | "htm")
        ) {
            return skip("an HTML document, not an asset");
        }
        let rel = path.strip_prefix(&self.site).ok()?;
        let public = format!(
            "/{}",
            rel.components()
                .map(|c| {
                    percent_encoding::utf8_percent_encode(&c.as_os_str().to_string_lossy(), SEGMENT)
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join("/")
        );
        let mut found = self.found.borrow_mut();
        if let Some(entry) = found.get_mut(&public) {
            if !entry.referenced_by.iter().any(|r| r == referrer) {
                entry.referenced_by.push(referrer.to_owned());
            }
            return Some(public);
        }
        if self.total.get() + meta.len() > MAX_TOTAL_BYTES {
            drop(found);
            return skip("the total of copied assets would exceed 768 MiB");
        }
        let Ok(bytes) = std::fs::read(&path) else {
            drop(found);
            return skip("unreadable");
        };
        self.total.set(self.total.get() + bytes.len() as u64);
        found.insert(
            public.clone(),
            AssetEntry {
                public_path: public.clone(),
                url: bare.to_string(),
                source: path
                    .strip_prefix(&self.root)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                bytes: bytes.len() as u64,
                sha256: hex(&Sha256::digest(&bytes)),
                referenced_by: vec![referrer.to_owned()],
            },
        );
        self.data.borrow_mut().insert(public.clone(), bytes);
        Some(public)
    }

    /// Resolve every `url(..)` in `css` for the project; returns how many were found.
    pub fn scan_css(&self, css: &str, base: &Url, referrer: &str) -> usize {
        let urls = css_urls(css);
        for u in &urls {
            self.resolve(u, base, referrer);
        }
        urls.len()
    }

    /// Copy the recorded assets: `(path under the project, bytes)`, plus the provenance rows.
    pub fn finish(self) -> Finished {
        let found = self.found.into_inner();
        let mut data = self.data.into_inner();
        let mut files = Vec::new();
        for e in found.values() {
            if let Some(bytes) = data.remove(&e.public_path) {
                files.push((format!("public{}", decode(&e.public_path)), bytes));
            }
        }
        let mut skipped = self.skipped.into_inner();
        skipped.sort_by(|a, b| (&a.url, &a.referenced_by).cmp(&(&b.url, &b.referenced_by)));
        skipped.dedup_by(|a, b| a.url == b.url && a.referenced_by == b.referenced_by);
        (files, found.into_values().collect(), skipped)
    }
}

/// Characters kept as they are in a served path segment.
const SEGMENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

fn decode(public: &str) -> String {
    percent_encoding::percent_decode_str(public)
        .decode_utf8_lossy()
        .into_owned()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Every `url(..)` target in a stylesheet, found with the CSS tokenizer (nested blocks, at-rules
/// and functions included), never by text matching.
pub fn css_urls(css: &str) -> Vec<String> {
    fn walk(parser: &mut Parser<'_>, out: &mut Vec<String>) {
        loop {
            let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
                break;
            };
            match token {
                Token::UnquotedUrl(url) => out.push(url.to_string()),
                Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                    let _ = parser.parse_nested_block(|p| {
                        loop {
                            match p.next() {
                                Ok(Token::QuotedString(s)) => {
                                    out.push(s.to_string());
                                    break;
                                }
                                Ok(Token::WhiteSpace(_)) => continue,
                                _ => break,
                            }
                        }
                        Ok::<(), ParseError<()>>(())
                    });
                }
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => {
                    let _ = parser.parse_nested_block(|p| {
                        walk(p, out);
                        Ok::<(), ParseError<()>>(())
                    });
                }
                _ => {}
            }
        }
    }
    let mut parser = Parser::new(css);
    let mut out = Vec::new();
    walk(&mut parser, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_resolve_to_the_mirrored_file_or_stay_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let site = dir.path().join("site");
        std::fs::create_dir_all(site.join("assets/img")).unwrap();
        std::fs::write(site.join("assets/img/a b.png"), b"png").unwrap();
        std::fs::write(site.join("page.html"), b"<html>").unwrap();
        let base = Url::parse("https://x.test/blog/post").unwrap();
        let a = Assets::new(dir.path(), &Url::parse("https://x.test/").unwrap());
        // Relative and absolute spellings of one file give one public path and one copy.
        assert_eq!(
            a.resolve("../assets/img/a%20b.png?v=3", &base, "route:/blog/post")
                .as_deref(),
            Some("/assets/img/a%20b.png")
        );
        assert_eq!(
            a.resolve("https://x.test/assets/img/a%20b.png", &base, "css:s.css")
                .as_deref(),
            Some("/assets/img/a%20b.png")
        );
        // Not resolvable here: other host, missing, data URI, an HTML page, a traversal attempt.
        assert_eq!(a.resolve("https://cdn.test/a.png", &base, "r"), None);
        assert_eq!(a.resolve("/assets/missing.png", &base, "r"), None);
        assert_eq!(a.resolve("data:image/png;base64,AAAA", &base, "r"), None);
        assert_eq!(a.resolve("/page.html", &base, "r"), None);
        assert_eq!(a.resolve("/../../etc/passwd", &base, "r"), None);
        let (files, entries, skipped) = a.finish();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "public/assets/img/a b.png");
        assert_eq!(files[0].1, b"png");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].referenced_by, ["route:/blog/post", "css:s.css"]);
        assert_eq!(entries[0].sha256.len(), 64);
        let reasons: Vec<&str> = skipped.iter().map(|s| s.reason.as_str()).collect();
        assert!(
            reasons.iter().any(|r| r.contains("another host")),
            "{reasons:?}"
        );
        assert!(
            reasons.iter().any(|r| r.contains("not present")),
            "{reasons:?}"
        );
        assert!(
            reasons.iter().any(|r| r.contains("HTML document")),
            "{reasons:?}"
        );
    }

    #[test]
    fn css_urls_come_from_tokens_not_text() {
        let css = r#"
            /* url(commented.png) */
            @font-face { font-family: A; src: url("/fonts/a.woff2") format("woff2"), url(/fonts/a.woff); }
            .x { background: image-set(url(a.png) 1x, url('b.png') 2x); content: "url(not-this.png)"; }
            @media (min-width: 1px) { .y { background: url( "c.svg" ) } }
        "#;
        let urls = css_urls(css);
        assert_eq!(
            urls,
            ["/fonts/a.woff2", "/fonts/a.woff", "a.png", "b.png", "c.svg"]
        );
    }
}
