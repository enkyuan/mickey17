use crate::{flight, util};
use anyhow::{Result, ensure};
use regex::Regex;
use std::path::Path;

pub fn apply(
    site: &Path,
    f: &impl Fn(&str) -> String,
    html: &impl Fn(&str) -> String,
) -> Result<usize> {
    apply_with_text(site, f, f, html)
}

fn apply_with_text(
    site: &Path,
    f: &impl Fn(&str) -> String,
    text_f: &impl Fn(&str) -> String,
    html: &impl Fn(&str) -> String,
) -> Result<usize> {
    let mut changed = 0;
    for p in util::text_files(site)? {
        let bytes = std::fs::read(&p)?;
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let ext = p
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let new = match ext.as_str() {
            "rsc" => flight::map_rows_with_text(text, f, text_f)?,
            "html" | "htm" => {
                flight::outside_flight(&flight::edit_flight_with_text(text, f, text_f)?, html)
            }
            _ => f(text),
        };
        if new != text {
            util::atomic_replace(&p, new.as_bytes())?;
            changed += 1;
        }
    }
    Ok(changed)
}

pub fn replace(site: &Path, replacer: &Replacer) -> Result<usize> {
    apply_with_text(
        site,
        &|s| replacer.replace_row(s),
        &|s| replacer.replace(s),
        &|s| replacer.replace(s),
    )
}

pub struct Replacer {
    protected: Regex,
    pairs: Vec<(Regex, String)>,
}
impl Replacer {
    pub fn new(old: &str, new: &str, variants: bool, whole: bool) -> Result<Self> {
        ensure!(!old.is_empty(), "replacement source must not be empty");
        let mut pairs = vec![(old.to_string(), new.to_string())];
        if variants {
            fn cap(s: &str) -> String {
                let mut cs = s.chars();
                cs.next().map_or_else(String::new, |c| {
                    c.to_uppercase().collect::<String>() + &cs.as_str().to_lowercase()
                })
            }
            for pair in [
                (old.to_uppercase(), new.to_uppercase()),
                (old.to_lowercase(), new.to_lowercase()),
                (cap(old), cap(new)),
            ] {
                if !pairs.iter().any(|(o, _)| *o == pair.0) {
                    pairs.push(pair);
                }
            }
        }
        Ok(Self {
            protected: Regex::new(
                r#"https?://[^\s"'\\<>)]+|^/[^\s"'\\<>)]*|["'(=]/[^\s"'\\<>)]*|(?:mailto|tel):[^\s"'\\<>]+"#,
            )?,
            pairs: pairs
                .into_iter()
                .map(|(o, n)| {
                    Ok((
                        Regex::new(&if whole {
                            format!(r"\b{}\b", regex::escape(&o))
                        } else {
                            regex::escape(&o)
                        })?,
                        n,
                    ))
                })
                .collect::<Result<_>>()?,
        })
    }
    pub fn replace_row(&self, row: &str) -> String {
        flight::map_json_strings(row, &|s| self.replace(s))
    }

    pub fn replace(&self, text: &str) -> String {
        let swap = |s: &str| {
            self.pairs.iter().fold(s.to_string(), |s, (r, n)| {
                r.replace_all(&s, regex::NoExpand(n)).into_owned()
            })
        };
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for m in self.protected.find_iter(text) {
            out.push_str(&swap(&text[last..m.start()]));
            out.push_str(m.as_str());
            last = m.end();
        }
        out.push_str(&swap(&text[last..]));
        out
    }
}

pub fn rehost(site: &Path, old: &str, new: &str) -> Result<usize> {
    let old = url::Url::parse(old)?;
    let new = url::Url::parse(new)?;
    ensure!(
        matches!(new.scheme(), "http" | "https"),
        "new origin must be HTTP(S)"
    );
    let host = old.host_str().unwrap_or("").trim_start_matches("www.");
    ensure!(!host.is_empty(), "missing old host");
    let rx = Regex::new(&format!(
        r"https?://(?:www\.)?{}{}",
        regex::escape(host),
        old.port().map_or_else(String::new, |p| format!(":{p}"))
    ))?;
    let origin = new.origin().ascii_serialization();
    let f = |s: &str| {
        let mut out = String::new();
        let mut last = 0;
        for m in rx.find_iter(s) {
            if s[m.end()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | ':'))
            {
                continue;
            }
            out.push_str(&s[last..m.start()]);
            out.push_str(&origin);
            last = m.end();
        }
        out.push_str(&s[last..]);
        out
    };
    apply(site, &f, &f)
}

pub fn localize(site: &Path, origin: &url::Url) -> Result<()> {
    let host = origin.host_str().unwrap_or("").trim_start_matches("www.");
    let rx = Regex::new(&format!(
        r"https?://(?:www\.)?{}{}(/)",
        regex::escape(host),
        origin.port().map_or_else(String::new, |p| format!(":{p}"))
    ))?;
    let asset =
        Regex::new(r"(?is)<(?:img|script|source|video|audio|track|link|image|use)\b[^>]*>")?;
    let block = Regex::new(
        r#"(?is)(<style\b[^>]*>|<script\b[^>]*type=["']?importmap[^>]*>)(.*?)(</style>|</script>)"#,
    )?;
    let doc =
        Regex::new(r#"(?i)rel=["']?(?:canonical|alternate|shortlink|prev|next|author|me)\b"#)?;
    for p in util::text_files(site)? {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "html" | "htm" | "css") {
            continue;
        }
        let bytes = std::fs::read(&p)?;
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let f = |s: &str| rx.replace_all(s, "/").into_owned();
        let html = |s: &str| {
            let s = asset.replace_all(s, |c: &regex::Captures<'_>| {
                if c[0].to_lowercase().starts_with("<link") && doc.is_match(&c[0]) {
                    c[0].to_string()
                } else {
                    f(&c[0])
                }
            });
            block
                .replace_all(&s, |c: &regex::Captures<'_>| {
                    format!("{}{}{}", &c[1], f(&c[2]), &c[3])
                })
                .into_owned()
        };
        let new = if ext == "css" {
            f(text)
        } else {
            flight::outside_flight(text, &html)
        };
        if new != text {
            util::atomic_replace(&p, new.as_bytes())?;
        }
    }
    Ok(())
}
