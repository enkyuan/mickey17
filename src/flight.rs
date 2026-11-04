//! React flight streams use byte lengths, not character counts.
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;
static PUSH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<script[^>]*>self\.__next_f\.push\((\[.*?\])\)</script>").expect("valid regex")
});
static TEXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9a-f]*):T([0-9a-f]+),").expect("valid regex"));

/// Edit complete rows and recompute UTF-8 lengths. Malformed text rows are rejected.
pub fn map_rows(stream: &str, f: &impl Fn(&str) -> String) -> Result<String> {
    map_rows_with_text(stream, f, f)
}

pub fn map_rows_with_text(
    stream: &str,
    f: &impl Fn(&str) -> String,
    text_f: &impl Fn(&str) -> String,
) -> Result<String> {
    let mut out = String::with_capacity(stream.len());
    let mut rest = stream;
    while !rest.is_empty() {
        if let Some(m) = TEXT.captures(rest) {
            let start = m.get(0).context("missing row header")?.end();
            let len = usize::from_str_radix(&m[2], 16)?;
            let end = start.checked_add(len).context("flight length overflow")?;
            let text = rest
                .get(start..end)
                .context("truncated or non UTF-8 flight row")?;
            let new = text_f(text);
            out.push_str(&format!("{}:T{:x},{}", &m[1], new.len(), new));
            rest = &rest[end..];
        } else {
            let end = rest.find('\n').map_or(rest.len(), |n| n + 1);
            out.push_str(&f(&rest[..end]));
            rest = &rest[end..];
        }
    }
    Ok(out)
}

/// Merge split data chunks before editing; preserve nonce and non-data pushes.
pub fn edit_flight(page: &str, f: &impl Fn(&str) -> String) -> Result<String> {
    edit_flight_with_text(page, f, f)
}

pub fn edit_flight_with_text(
    page: &str,
    f: &impl Fn(&str) -> String,
    text_f: &impl Fn(&str) -> String,
) -> Result<String> {
    let mut chunks = Vec::new();
    let mut stream = String::new();
    for cap in PUSH.captures_iter(page) {
        let arr: Value = serde_json::from_str(&cap[1])?;
        if arr[0] == 1
            && let Some(text) = arr[1].as_str()
        {
            stream.push_str(text);
            chunks.push(cap.get(0).context("missing chunk")?);
        }
    }
    if chunks.is_empty() {
        return Ok(page.into());
    }
    let new = map_rows_with_text(&stream, f, text_f)?;
    if new == stream {
        return Ok(page.into());
    }
    let enc = serde_json::to_string(&serde_json::json!([1, new]))?
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    let mut out = String::with_capacity(page.len());
    let mut last = 0;
    for (i, m) in chunks.iter().enumerate() {
        out.push_str(&page[last..m.start()]);
        if i == 0 {
            let Some(tag_end) = m.as_str().find('>') else {
                bail!("invalid script tag")
            };
            out.push_str(&m.as_str()[..=tag_end]);
            out.push_str(&format!("self.__next_f.push({enc})</script>"));
        }
        last = m.end();
    }
    out.push_str(&page[last..]);
    Ok(out)
}

pub fn outside_flight(page: &str, f: &impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(page.len());
    let mut last = 0;
    for m in PUSH.find_iter(page) {
        out.push_str(&f(&page[last..m.start()]));
        out.push_str(m.as_str());
        last = m.end();
    }
    out.push_str(&f(&page[last..]));
    out
}

/// Rewrite string values in JSON rows while keeping JSON quoting valid.
/// Non-JSON text (including long text-row bodies) is passed through directly.
pub fn map_json_strings(row: &str, f: &impl Fn(&str) -> String) -> String {
    let Some((id, body)) = row.split_once(':') else {
        return f(row);
    };
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return f(row);
    }
    let (tag, json) = if body.starts_with(|c: char| c.is_ascii_uppercase()) {
        (&body[..1], &body[1..])
    } else {
        ("", body)
    };
    let Ok(mut tree) = serde_json::from_str::<Value>(json) else {
        return f(row);
    };
    let old = tree.clone();
    fn walk(v: &mut Value, f: &impl Fn(&str) -> String) {
        match v {
            Value::String(s) => *s = f(s),
            Value::Array(a) => a.iter_mut().for_each(|v| walk(v, f)),
            Value::Object(o) => o.values_mut().for_each(|v| walk(v, f)),
            _ => {}
        }
    }
    walk(&mut tree, f);
    if tree == old {
        row.into()
    } else {
        format!(
            "{id}:{tag}{tree}{}",
            if row.ends_with('\n') { "\n" } else { "" }
        )
    }
}

pub fn json_row(text: &str, f: &impl Fn(&mut Value)) -> String {
    let Some((id, body)) = text.split_once(':') else {
        return text.into();
    };
    if !id.chars().all(|c| c.is_ascii_hexdigit()) || !body.starts_with(['[', '{']) {
        return text.into();
    }
    let Ok(mut tree) = serde_json::from_str::<Value>(body) else {
        return text.into();
    };
    let old = tree.clone();
    fn walk(v: &mut Value, f: &impl Fn(&mut Value)) {
        f(v);
        match v {
            Value::Array(a) => a.iter_mut().for_each(|v| walk(v, f)),
            Value::Object(o) => o.values_mut().for_each(|v| walk(v, f)),
            _ => {}
        }
    }
    walk(&mut tree, f);
    if old == tree {
        text.into()
    } else {
        format!(
            "{id}:{tree}{}",
            if text.ends_with('\n') { "\n" } else { "" }
        )
    }
}
