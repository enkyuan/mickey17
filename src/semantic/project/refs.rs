//! A check on the finished file set, independent of how the files were produced: every
//! root-relative asset URL that the emitted sources and stylesheets mention must be a file under
//! `public/`. The asset resolver records what it was asked to copy; this records what the output
//! actually points at, so a reference that bypassed the resolver is counted as not copied instead
//! of being reported as complete.
use super::assets::css_urls;
use oxc_allocator::Allocator;
use oxc_ast::ast::StringLiteral;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;
use std::collections::{BTreeMap, BTreeSet};

/// File extensions that denote an asset the browser fetches (not a route).
const ASSET_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "ico", "bmp", "apng", "mp4", "webm", "ogv",
    "mov", "mp3", "ogg", "wav", "m4a", "woff", "woff2", "ttf", "otf", "eot", "json", "wasm", "glb",
    "gltf", "hdr", "ktx2", "pdf", "txt", "xml", "css", "js", "mjs",
];

/// One reference that has no file behind it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dangling {
    /// The project file that holds the reference.
    pub file: String,
    /// The root-relative URL as written.
    pub url: String,
}

struct Strings(Vec<String>);

impl<'a> Visit<'a> for Strings {
    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        if let Some(v) = it.value.as_str() {
            self.0.push(v.to_owned());
        }
        walk::walk_string_literal(self, it);
    }
}

fn is_asset_url(u: &str) -> bool {
    u.starts_with('/')
        && !u.starts_with("//")
        && u.split(['?', '#'])
            .next()
            .and_then(|path| path.rsplit_once('.'))
            .is_some_and(|(_, ext)| ASSET_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// The root-relative asset URLs inside one string value: the value itself, the candidates of a
/// `srcSet` (separated by a comma and a space; a comma inside a URL stays), and `url(..)` inside
/// an arbitrary-value class.
fn candidates(value: &str) -> Vec<String> {
    let value = value.trim();
    if !value.contains(char::is_whitespace) && is_asset_url(value) {
        return vec![value.to_owned()];
    }
    let mut out: Vec<String> = Vec::new();
    if value.contains("url(") {
        out.extend(css_urls(value).into_iter().filter(|u| is_asset_url(u)));
    }
    out.extend(
        value
            .split(", ")
            .filter_map(|part| part.split_whitespace().next())
            .filter(|u| is_asset_url(u))
            .map(str::to_owned),
    );
    out
}

fn served(files: &BTreeMap<String, Vec<u8>>, url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let decoded = percent_encoding::percent_decode_str(path).decode_utf8_lossy();
    files.contains_key(&format!("public{decoded}"))
}

/// Root-relative asset references in `.tsx` sources and `.css` files that are not under `public/`.
pub fn dangling(files: &BTreeMap<String, Vec<u8>>) -> Vec<Dangling> {
    let mut out = BTreeSet::new();
    for (path, bytes) in files {
        if !path.starts_with("src/") {
            continue;
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        let urls: Vec<String> = if path.ends_with(".tsx") {
            let arena = Allocator::default();
            let parsed = Parser::new(&arena, text, SourceType::tsx()).parse();
            let mut strings = Strings(Vec::new());
            strings.visit_program(&parsed.program);
            strings.0.iter().flat_map(|s| candidates(s)).collect()
        } else if path.ends_with(".css") {
            css_urls(text).iter().flat_map(|u| candidates(u)).collect()
        } else {
            continue;
        };
        for url in urls {
            if !served(files, &url) {
                out.insert(Dangling {
                    file: path.clone(),
                    url,
                });
            }
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(entries: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn reports_asset_references_without_a_file_and_ignores_the_rest() {
        let f = files(&[
            (
                "src/pages/A.tsx",
                r#"export const A = () => (<div><img src="/c/w_1,h_1/x.png" /><i className="bg-[url(/k/bg.png)]" /><img src="/a/x.png" /><img src="/a/missing.jpg" srcSet="/a/x.png 1x, /a/m%202.webp 2x" /><a href="/pay">Pay, now.png</a><p title="//cdn.test/q.png">t</p></div>)"#,
            ),
            ("public/a/x.png", ""),
            (
                "src/styles/s.css",
                ".a{background:url(/f/gone.woff2)} .b{background:url(/a/x.png)}",
            ),
        ]);
        let got: Vec<(String, String)> =
            dangling(&f).into_iter().map(|d| (d.file, d.url)).collect();
        assert_eq!(
            got,
            [
                ("src/pages/A.tsx".to_owned(), "/a/m%202.webp".to_owned()),
                ("src/pages/A.tsx".to_owned(), "/a/missing.jpg".to_owned()),
                ("src/pages/A.tsx".to_owned(), "/c/w_1,h_1/x.png".to_owned()),
                ("src/pages/A.tsx".to_owned(), "/k/bg.png".to_owned()),
                ("src/styles/s.css".to_owned(), "/f/gone.woff2".to_owned()),
            ]
        );
    }
}
