//! Document-head evidence that the DOM snapshot does not carry.
//!
//! A saved capture serializes `<head>` as only its `<title>`; icon links and
//! JSON-LD scripts exist solely in the mirrored HTML response that sits beside
//! the snapshots. This module reads that response, never executes it, and
//! returns only passive, whitelisted head data.
use crate::util;
use anyhow::Result;
use scraper::{Html, Selector};
use std::{collections::BTreeMap, io::Read, path::Path};
use url::Url;

const HTML_BYTES: u64 = 16 * 1024 * 1024;
const ICON_RELS: [&str; 5] = [
    "icon",
    "shortcut",
    "apple-touch-icon",
    "apple-touch-icon-precomposed",
    "mask-icon",
];
/// Link relations that only wire the original framework's runtime or loading.
const RUNTIME_RELS: [&str; 7] = [
    "stylesheet",
    "preload",
    "modulepreload",
    "prefetch",
    "dns-prefetch",
    "preconnect",
    "canonical",
];

#[derive(Debug, Default)]
pub(super) struct Head {
    /// Icon links with their original (unresolved) `href`.
    pub icons: Vec<BTreeMap<String, String>>,
    /// Validated JSON documents from `application/ld+json` scripts, in author order.
    pub json_ld: Vec<String>,
    /// Human-readable reasons for links/scripts that were deliberately not exported.
    pub skipped: Vec<String>,
    /// Number of byte-identical icon declarations collapsed into one.
    pub duplicates: usize,
}

/// A mirrored file that influenced the export, with the digest of the bytes read.
pub(super) struct Input {
    pub text: String,
    /// Path relative to the capture root.
    pub path: String,
    pub sha256: String,
}
/// The mirrored HTML response for a page URL, when the capture holds one.
pub(super) fn read(root: &Path, page: &Url) -> Result<Option<Input>> {
    let site = root.join("site");
    let mut without_query = page.clone();
    without_query.set_query(None);
    without_query.set_fragment(None);
    let Ok(path) = crate::mirror::output_path(&site, &without_query, true)
        .and_then(|path| util::checked_path(&site, &path))
    else {
        return Ok(None);
    };
    let Ok(file) = std::fs::File::open(&path) else {
        return Ok(None);
    };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > HTML_BYTES {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.take(HTML_BYTES + 1).read_to_end(&mut bytes)?;
    let sha256 = crate::cache::fingerprint(&bytes);
    let relative = path
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(String::from_utf8(bytes).ok().map(|text| Input {
        text,
        path: relative,
        sha256,
    }))
}

pub(super) fn parse(html: &str) -> Head {
    let document = Html::parse_document(html);
    let mut head = Head::default();
    let links = Selector::parse("link[rel]").expect("link selector");
    let mut seen = Vec::new();
    for link in document.select(&links) {
        let element = link.value();
        let rel = element.attr("rel").unwrap_or("").to_ascii_lowercase();
        let tokens: Vec<&str> = rel.split_ascii_whitespace().collect();
        if tokens.iter().any(|t| RUNTIME_RELS.contains(t)) {
            continue;
        }
        let Some(href) = element.attr("href").filter(|h| !h.trim().is_empty()) else {
            continue;
        };
        if !tokens.iter().any(|t| ICON_RELS.contains(t)) {
            head.skipped.push(format!("link rel={rel} href={href}"));
            continue;
        }
        let mut attrs = BTreeMap::new();
        attrs.insert("rel".to_owned(), rel.clone());
        attrs.insert("href".to_owned(), href.trim().to_owned());
        for name in ["type", "sizes", "media", "color"] {
            if let Some(value) = element.attr(name) {
                attrs.insert(name.to_owned(), value.to_owned());
            }
        }
        if seen.contains(&attrs) {
            head.duplicates += 1;
        } else {
            seen.push(attrs.clone());
            head.icons.push(attrs);
        }
    }
    let scripts = Selector::parse("script[type]").expect("script selector");
    for script in document.select(&scripts) {
        let kind = script.value().attr("type").unwrap_or("");
        if !kind
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/ld+json")
        {
            continue;
        }
        let text: String = script.text().collect();
        // Validate, but keep the author's key order and values: re-serializing
        // through serde_json would sort keys.
        match serde_json::from_str::<serde::de::IgnoredAny>(&text) {
            Ok(_) => head.json_ld.push(text.trim().to_owned()),
            Err(error) => head
                .skipped
                .push(format!("invalid application/ld+json ({error})")),
        }
    }
    head
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn icons_and_json_ld_are_kept_and_runtime_links_are_not() {
        let head = parse(
            r#"<html><head>
            <link rel="stylesheet" href="/a.css"><link rel="preload" as="image" href="/p.png">
            <link rel="canonical" href="https://example.test/">
            <link rel="icon" href="/favicon.ico"><link rel="icon" href="/favicon.ico" nonce="x">
            <link rel="apple-touch-icon" sizes="180x180" href="/touch.png">
            <link rel="manifest" href="/site.webmanifest">
            <script type="application/ld+json">{"@type":"Organization","name":"Example"}</script>
            <script type="application/ld+json">{broken</script>
            <script>window.runtime=1</script></head><body></body></html>"#,
        );
        assert_eq!(head.icons.len(), 2);
        assert_eq!(head.duplicates, 1);
        assert_eq!(head.icons[1]["sizes"], "180x180");
        assert_eq!(head.json_ld.len(), 1);
        assert!(head.json_ld[0].contains("Organization"));
        // Author key order survives (serde_json would sort "@type" before "name" anyway
        // but reorder "name","url","logo" to "logo","name","url").
        let ordered = parse(
            r#"<script type="application/ld+json; charset=utf-8">{"name":"N","url":"u","logo":"l"}</script>"#,
        );
        assert_eq!(ordered.json_ld, [r#"{"name":"N","url":"u","logo":"l"}"#]);
        assert!(head.skipped.iter().any(|s| s.contains("manifest")));
        assert!(head.skipped.iter().any(|s| s.contains("invalid")));
    }
}
