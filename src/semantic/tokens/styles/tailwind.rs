//! The CSS the generated project's utilities produce: compiled by a real Tailwind v4 when one
//! is installed (the oracle for the equality check), otherwise written from this crate's own
//! model of the utilities it emits.

use super::classes::{self, Theme};
use super::layers::State;
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SCRIPT: &str = include_str!("../../../../scripts/tokens/tailwind-compile.mjs");

/// Find a Tailwind package: `MOCHI_TAILWIND_DIR`, or the first `node_modules/tailwindcss`
/// of an export below `.mochi/generated/exports` (relative to the working directory).
pub fn locate() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("MOCHI_TAILWIND_DIR") {
        let p = PathBuf::from(dir);
        return p.join("dist/lib.mjs").is_file().then_some(p);
    }
    let exports = Path::new(".mochi/generated/exports");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(exports)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    entries
        .into_iter()
        .map(|p| p.join("node_modules/tailwindcss"))
        .find(|p| p.join("dist/lib.mjs").is_file())
}

pub fn version(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["version"].as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
}

/// Is `bun` (which runs the Tailwind compiler) on the PATH?
pub fn bun_available() -> bool {
    Command::new("bun")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Class names (from `names`) for which Tailwind generates CSS with an empty theme: static
/// utilities such as `collapse` or `list-item`. A recipe with such a name would be styled twice.
pub fn generated_class_names(
    dir: &Path,
    names: &[String],
) -> Result<std::collections::BTreeSet<String>> {
    use super::super::css::{self, Item, Node};
    use super::super::selector::Simple;
    fn walk(nodes: &[Node], out: &mut std::collections::BTreeSet<String>) {
        for node in nodes {
            match node {
                Node::Style(rule) => {
                    for c in &rule.selectors.0 {
                        for k in &c.compounds {
                            for s in &k.simples {
                                if let Simple::Class(n) = s {
                                    out.insert(n.clone());
                                }
                            }
                        }
                    }
                }
                Node::At(at) => {
                    if let Some(items) = &at.items {
                        let inner: Vec<Node> = items
                            .iter()
                            .filter_map(|i| match i {
                                Item::Rule(n) => Some(n.clone()),
                                Item::Decl(_) => None,
                            })
                            .collect();
                        walk(&inner, out);
                    }
                }
                _ => {}
            }
        }
    }
    let compiled = compile(
        dir,
        "@theme static { --spacing: 4px; }\n@tailwind utilities;\n",
        names,
    )?;
    let mut found = std::collections::BTreeSet::new();
    walk(&css::parse_sheet(&compiled).nodes, &mut found);
    Ok(found.into_iter().filter(|n| names.contains(n)).collect())
}

/// Compile `css` (which must contain `@tailwind utilities;`) with `candidates`.
pub fn compile(dir: &Path, css: &str, candidates: &[String]) -> Result<String> {
    let script = tempfile::Builder::new()
        .suffix(".mjs")
        .tempfile()
        .context("temp script")?;
    std::fs::write(script.path(), SCRIPT)?;
    let dir = dir
        .canonicalize()
        .with_context(|| format!("tailwind dir {}", dir.display()))?;
    let mut child = Command::new("bun")
        .arg(script.path())
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("run bun")?;
    let payload = serde_json::json!({"css": css, "candidates": candidates}).to_string();
    // A failing script exits before reading its input; the error then comes from its stderr.
    let _ = child
        .stdin
        .take()
        .context("stdin")?
        .write_all(payload.as_bytes());
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "tailwind compile failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn escape_selector(class: &str) -> String {
    let mut out = String::new();
    for (i, c) in class.chars().enumerate() {
        if c.is_ascii_alphanumeric() && !(i == 0 && c.is_ascii_digit()) || c == '-' || c == '_' {
            out.push(c);
        } else if i == 0 && c.is_ascii_digit() {
            out.push_str(&format!("\\3{c} "));
        } else {
            out.push('\\');
            out.push(c);
        }
    }
    out
}

/// A class split into its variants and body.
#[derive(Debug, PartialEq, Eq)]
pub struct Parsed<'a> {
    pub min: Option<u32>,
    pub state: Option<State>,
    pub body: &'a str,
}

pub fn parse_class(class: &str) -> Option<Parsed<'_>> {
    let mut rest = class;
    let mut min = None;
    let mut state = None;
    loop {
        // Variant separators are the colons outside `[...]`.
        let mut depth = 0usize;
        let mut cut = None;
        for (i, c) in rest.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => depth = depth.saturating_sub(1),
                ':' if depth == 0 => {
                    cut = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let Some(i) = cut else { break };
        let variant = &rest[..i];
        if let Some(w) = variant
            .strip_prefix("min-[")
            .and_then(|v| v.strip_suffix("px]"))
        {
            if min.is_some() {
                return None;
            }
            min = Some(w.parse().ok()?);
        } else {
            let s = State::ALL.into_iter().find(|s| s.variant() == variant)?;
            if state.is_some() {
                return None;
            }
            state = Some(s);
        }
        rest = &rest[i + 1..];
    }
    Some(Parsed {
        min,
        state,
        body: rest,
    })
}

fn rule(selector: &str, decls: &[(String, String)]) -> String {
    let mut out = format!("{selector} {{\n");
    for (p, v) in decls {
        out.push_str(&format!("  {p}: {v};\n"));
    }
    out.push_str("}\n");
    out
}

/// The model of one utility's CSS: `selector` is the class selector (`.p-4`, `.recipe`).
/// Variants nest the way Tailwind's output does: state inside `@media (hover: hover)` (hover
/// only), width media outside.
fn utility_rule(selector: &str, p: &Parsed<'_>, decls: &[(String, String)]) -> String {
    let sel = match p.state {
        Some(State::ReducedMotion) | None => selector.to_string(),
        Some(s) => format!("{selector}:{}", s.variant()),
    };
    let mut css = rule(&sel, decls);
    if p.state == Some(State::Hover) {
        css = format!("@media (hover: hover) {{\n{}}}\n", indent(&css));
    }
    if p.state == Some(State::ReducedMotion) {
        css = format!(
            "@media (prefers-reduced-motion: reduce) {{\n{}}}\n",
            indent(&css)
        );
    }
    if let Some(w) = p.min {
        css = format!("@media (width >= {w}px) {{\n{}}}\n", indent(&css));
    }
    css
}

fn indent(css: &str) -> String {
    css.lines().map(|l| format!("  {l}\n")).collect()
}

/// Order key of Tailwind's output: plain utilities, then states, then widths ascending, with
/// a state inside a width after the width's plain utilities (observed on Tailwind 4.3.5).
fn order_key(p: &Parsed<'_>) -> (u32, u8, u32) {
    let state_rank = p.state.map_or(0u8, |s| match s {
        State::FocusWithin => 1,
        State::Hover => 2,
        State::Focus => 3,
        State::FocusVisible => 4,
        State::Active => 5,
        State::ReducedMotion => 6,
    });
    match p.min {
        None => (0, state_rank, 0),
        Some(w) => (1 + w, state_rank, w),
    }
}

/// Utilities CSS written from [`classes::expand`]: `uses` are `(selector, class)` pairs
/// (`(".p-4", "p-4")`, or a recipe's selector with each class it applies). `None` if a class
/// is not one this model knows.
pub fn modelled(uses: &[(String, String)], theme: &Theme, theme_css: &str) -> Option<String> {
    let mut items: Vec<((u32, u8, u32), String)> = Vec::new();
    for (selector, class) in uses {
        let parsed = parse_class(class)?;
        let decls = classes::expand(parsed.body, theme)?;
        items.push((order_key(&parsed), utility_rule(selector, &parsed, &decls)));
    }
    items.sort_by_key(|a| a.0);
    let mut out = String::from(theme_css);
    for (_, css) in items {
        out.push_str(&css);
    }
    Some(out)
}

/// `.p-4` style selector for a utility class.
pub fn class_selector(class: &str) -> String {
    format!(".{}", escape_selector(class))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every utility this crate renders, compiled by a real Tailwind (skipped when none is installed):
    /// the declarations Tailwind generates must be exactly the ones `classes::expand` claims.
    #[test]
    fn utilities_expand_exactly_like_a_real_tailwind() {
        use super::super::super::css;
        let Some(dir) = locate() else {
            eprintln!("skipped: no Tailwind package (set MOCHI_TAILWIND_DIR)");
            return;
        };
        let names = [
            "--spacing",
            "--color-ink",
            "--color-white",
            "--radius-card",
            "--font-sans",
        ]
        .map(String::from);
        let theme = Theme::from_definitions(names.iter(), Some(4.0));
        let theme_css = "@theme static { --spacing: 4px; --color-ink: #111; --color-white: #fff; --radius-card: 12px; --font-sans: Arial; }";
        let mut classes: Vec<String> = classes::KEYWORDS
            .iter()
            .map(|(c, _, _)| (*c).to_string())
            .collect();
        for c in [
            "p-4",
            "pt-2",
            "pr-1",
            "pb-0",
            "pl-2.5",
            "m-auto",
            "-mt-4",
            "ml-auto",
            "gap-4",
            "gap-x-2",
            "gap-y-0.5",
            "inset-0",
            "top-1",
            "-left-2",
            "w-full",
            "w-fit",
            "w-min",
            "w-max",
            "w-auto",
            "w-screen",
            "h-full",
            "h-screen",
            "h-auto",
            "min-w-0",
            "min-h-full",
            "max-w-none",
            "max-w-full",
            "max-h-screen",
            "text-ink",
            "bg-white",
            "border-ink",
            "border-t-ink",
            "outline-ink",
            "fill-ink",
            "stroke-ink",
            "caret-ink",
            "accent-ink",
            "decoration-ink",
            "rounded-card",
            "rounded-tl-card",
            "rounded-br-card",
            "font-sans",
            "opacity-37",
            "opacity-0",
            "opacity-100",
            "z-10",
            "z-auto",
            "z-0",
        ] {
            classes.push(c.to_string());
        }
        let mut mismatches: Vec<String> = Vec::new();
        for class in &classes {
            let Some(expected) = classes::expand(class, &theme) else {
                mismatches.push(format!("{class}: no expansion"));
                continue;
            };
            let input = format!("{theme_css}\n@tailwind utilities;\n");
            let compiled = match compile(&dir, &input, std::slice::from_ref(class)) {
                Ok(c) => c,
                Err(e) => {
                    mismatches.push(format!("{class}: {e}"));
                    continue;
                }
            };
            let sheet = css::parse_sheet(&compiled);
            let mut got: Vec<(String, String)> = Vec::new();
            for node in &sheet.nodes {
                if let css::Node::Style(rule) = node {
                    if rule.source.starts_with(':') {
                        continue; // the theme
                    }
                    for item in &rule.items {
                        if let css::Item::Decl(d) = item {
                            got.push((d.name.clone(), css::to_css(&css::trim(&d.value))));
                        }
                    }
                }
            }
            if got.is_empty() {
                mismatches.push(format!("{class}: Tailwind produced nothing"));
                continue;
            }
            let norm = |v: &str| v.replace(' ', "");
            let mut want: Vec<(String, String)> =
                expected.iter().map(|(p, v)| (p.clone(), norm(v))).collect();
            let mut have: Vec<(String, String)> =
                got.iter().map(|(p, v)| (p.clone(), norm(v))).collect();
            want.sort();
            have.sort();
            if want != have {
                mismatches.push(format!("{class}: model {want:?} vs Tailwind {have:?}"));
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }

    #[test]
    fn variants_are_split_outside_brackets() {
        assert_eq!(
            parse_class("min-[900px]:hover:bg-ink"),
            Some(Parsed {
                min: Some(900),
                state: Some(State::Hover),
                body: "bg-ink"
            })
        );
        assert_eq!(
            parse_class("[background:url(a:b)]"),
            Some(Parsed {
                min: None,
                state: None,
                body: "[background:url(a:b)]"
            })
        );
        assert_eq!(parse_class("md:p-4"), None);
        assert_eq!(parse_class("hover:focus:p-4"), None);
    }

    #[test]
    fn modelled_rules_nest_like_tailwind_and_sort_by_width() {
        let theme = Theme::from_definitions(["--color-ink".to_string()].iter(), Some(4.0));
        let uses: Vec<(String, String)> =
            ["min-[900px]:p-4", "p-2", "hover:p-3", "min-[600px]:p-8"]
                .iter()
                .map(|c| (class_selector(c), (*c).to_string()))
                .collect();
        let css = modelled(&uses, &theme, "").unwrap();
        let positions: Vec<usize> = [
            ".p-2",
            ".hover\\:p-3:hover",
            "min-\\[600px\\]",
            "min-\\[900px\\]",
        ]
        .iter()
        .map(|n| css.find(n).unwrap_or_else(|| panic!("{n} in {css}")))
        .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{css}");
        assert!(
            css.contains("@media (hover: hover) {\n  .hover\\:p-3:hover {"),
            "{css}"
        );
    }

    #[test]
    fn selectors_escape_everything_unsafe() {
        assert_eq!(escape_selector("[padding:7px]"), "\\[padding\\:7px\\]");
        assert_eq!(escape_selector("-ml-2.5"), "-ml-2\\.5");
        assert_eq!(escape_selector("2xl:p-4"), "\\32 xl\\:p-4");
    }
}
