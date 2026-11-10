//! Document-head evidence for a page that the DOM snapshot does not carry (icon links, JSON-LD),
//! read from the mirrored HTML response and never executed. This mirrors the whitelist of
//! `export::head` (that module is private to the compat exporter and under edit); keeping the
//! rules identical keeps both generators' output comparable.
use crate::{mirror, util};
use scraper::{Html, Selector};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
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
pub struct Head {
    /// Icon links with their original (unresolved) `href`.
    pub icons: Vec<BTreeMap<String, String>>,
    /// Validated JSON documents from `application/ld+json` scripts, in author order.
    pub json_ld: Vec<String>,
    /// Links and scripts deliberately not carried over, with the reason.
    pub skipped: Vec<String>,
}

/// The mirrored HTML response of a page, when the capture holds one.
pub fn read(capture_dir: &Path, page: &Url) -> Option<String> {
    let site = capture_dir.join("site");
    let mut bare = page.clone();
    bare.set_query(None);
    bare.set_fragment(None);
    let path = mirror::output_path(&site, &bare, true)
        .and_then(|p| util::checked_path(&site, &p))
        .ok()?;
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() || file.metadata().ok()?.len() > HTML_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(HTML_BYTES + 1).read_to_end(&mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

pub fn parse(html: &str) -> Head {
    let document = Html::parse_document(html);
    let mut head = Head::default();
    let links = Selector::parse("link[rel]").expect("link selector");
    let mut seen: Vec<BTreeMap<String, String>> = Vec::new();
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
        if !seen.contains(&attrs) {
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
        // Validate, but keep the author's key order and values.
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
            r#"<html><head><link rel="stylesheet" href="/a.css"><link rel="icon" href="/f.ico"><link rel="icon" href="/f.ico">
            <link rel="apple-touch-icon" sizes="180x180" href="/t.png"><link rel="manifest" href="/m.json">
            <script type="application/ld+json">{"name":"N","url":"u"}</script><script type="application/ld+json">{bad</script></head></html>"#,
        );
        assert_eq!(head.icons.len(), 2);
        assert_eq!(head.icons[1]["sizes"], "180x180");
        assert_eq!(head.json_ld, [r#"{"name":"N","url":"u"}"#]);
        assert!(head.skipped.iter().any(|s| s.contains("manifest")));
        assert!(head.skipped.iter().any(|s| s.contains("invalid")));
    }
}
