//! Declaration <-> Tailwind v4 utility.
//!
//! Every utility this module renders has an exact expansion (`expand`) back to the
//! declarations it stands for; `render` is only allowed to pick a utility whose expansion is
//! the declaration itself. Anything without such a utility becomes an arbitrary property
//! (`[prop:value]`), which Tailwind defines as exactly that declaration. The tables are
//! checked against a real Tailwind build by `tailwind::tests` (skipped when none is found).
//!
//! Deliberately not used because Tailwind's utility adds more than the declaration: `font-*`
//! weights, `leading-*`, `tracking-*`, `shadow-*`, `ring`, `border`, `transition`, `text-<size>`
//! (sets line-height too), `px`/`py` (logical properties), `select-*` (adds the -webkit- prefix).

use super::super::css::{self, Component, Tok};
use std::collections::BTreeSet;

/// Names the project's theme (the central tokens file) defines, by Tailwind namespace.
#[derive(Debug, Default, Clone)]
pub struct Theme {
    pub colors: BTreeSet<String>,
    pub radii: BTreeSet<String>,
    pub fonts: BTreeSet<String>,
    /// `--text-*` keys: a color named like one would be ambiguous for `text-<name>`.
    pub text_sizes: BTreeSet<String>,
    /// px per `--spacing` step when a scale exists.
    pub spacing_base_px: Option<f64>,
    /// Keys the theme defines in the spacing-like namespaces (`--spacing-0`, `--container-full`):
    /// Tailwind then resolves `p-0`, `h-full`, ... to `var(--spacing-0)` instead of the built-in
    /// value, so those keys must not be written as named utilities.
    pub shadowed_keys: BTreeSet<String>,
}

/// Theme namespaces that feed the spacing/sizing utilities.
const SIZING_NAMESPACES: &[&str] = &[
    "--spacing-",
    "--inset-",
    "--margin-",
    "--padding-",
    "--gap-",
    "--width-",
    "--height-",
    "--size-",
    "--container-",
    "--max-width-",
    "--max-height-",
    "--min-width-",
    "--min-height-",
];

impl Theme {
    /// Read the names out of the central file's definitions.
    pub fn from_definitions<'a>(
        names: impl Iterator<Item = &'a String>,
        spacing_base_px: Option<f64>,
    ) -> Theme {
        let mut theme = Theme {
            spacing_base_px,
            ..Default::default()
        };
        for name in names {
            if let Some(key) = SIZING_NAMESPACES
                .iter()
                .find_map(|ns| name.strip_prefix(ns))
            {
                theme.shadowed_keys.insert(key.to_string());
            }
            if let Some(n) = name.strip_prefix("--color-") {
                theme.colors.insert(n.to_string());
            } else if let Some(n) = name.strip_prefix("--radius-") {
                theme.radii.insert(n.to_string());
            } else if let Some(n) = name.strip_prefix("--text-") {
                theme.text_sizes.insert(n.to_string());
            } else if let Some(n) = name.strip_prefix("--font-")
                && !n.starts_with("weight-")
            {
                theme.fonts.insert(n.to_string());
            }
        }
        theme
    }
}

/// `(class, property, value)` utilities whose expansion is exactly one declaration.
pub const KEYWORDS: &[(&str, &str, &str)] = &[
    ("block", "display", "block"),
    ("inline-block", "display", "inline-block"),
    ("inline", "display", "inline"),
    ("flex", "display", "flex"),
    ("inline-flex", "display", "inline-flex"),
    ("grid", "display", "grid"),
    ("inline-grid", "display", "inline-grid"),
    ("hidden", "display", "none"),
    ("contents", "display", "contents"),
    ("flow-root", "display", "flow-root"),
    ("table", "display", "table"),
    ("static", "position", "static"),
    ("relative", "position", "relative"),
    ("absolute", "position", "absolute"),
    ("fixed", "position", "fixed"),
    ("sticky", "position", "sticky"),
    ("visible", "visibility", "visible"),
    ("invisible", "visibility", "hidden"),
    ("overflow-auto", "overflow", "auto"),
    ("overflow-hidden", "overflow", "hidden"),
    ("overflow-visible", "overflow", "visible"),
    ("overflow-scroll", "overflow", "scroll"),
    ("overflow-clip", "overflow", "clip"),
    ("overflow-x-auto", "overflow-x", "auto"),
    ("overflow-x-hidden", "overflow-x", "hidden"),
    ("overflow-x-visible", "overflow-x", "visible"),
    ("overflow-x-scroll", "overflow-x", "scroll"),
    ("overflow-x-clip", "overflow-x", "clip"),
    ("overflow-y-auto", "overflow-y", "auto"),
    ("overflow-y-hidden", "overflow-y", "hidden"),
    ("overflow-y-visible", "overflow-y", "visible"),
    ("overflow-y-scroll", "overflow-y", "scroll"),
    ("overflow-y-clip", "overflow-y", "clip"),
    ("flex-row", "flex-direction", "row"),
    ("flex-row-reverse", "flex-direction", "row-reverse"),
    ("flex-col", "flex-direction", "column"),
    ("flex-col-reverse", "flex-direction", "column-reverse"),
    ("flex-wrap", "flex-wrap", "wrap"),
    ("flex-nowrap", "flex-wrap", "nowrap"),
    ("flex-wrap-reverse", "flex-wrap", "wrap-reverse"),
    ("grow", "flex-grow", "1"),
    ("grow-0", "flex-grow", "0"),
    ("shrink", "flex-shrink", "1"),
    ("shrink-0", "flex-shrink", "0"),
    ("items-start", "align-items", "flex-start"),
    ("items-end", "align-items", "flex-end"),
    ("items-center", "align-items", "center"),
    ("items-baseline", "align-items", "baseline"),
    ("items-stretch", "align-items", "stretch"),
    ("justify-start", "justify-content", "flex-start"),
    ("justify-end", "justify-content", "flex-end"),
    ("justify-center", "justify-content", "center"),
    ("justify-between", "justify-content", "space-between"),
    ("justify-around", "justify-content", "space-around"),
    ("justify-evenly", "justify-content", "space-evenly"),
    ("justify-stretch", "justify-content", "stretch"),
    ("justify-normal", "justify-content", "normal"),
    ("self-auto", "align-self", "auto"),
    ("self-start", "align-self", "flex-start"),
    ("self-end", "align-self", "flex-end"),
    ("self-center", "align-self", "center"),
    ("self-stretch", "align-self", "stretch"),
    ("self-baseline", "align-self", "baseline"),
    ("text-left", "text-align", "left"),
    ("text-center", "text-align", "center"),
    ("text-right", "text-align", "right"),
    ("text-justify", "text-align", "justify"),
    ("text-start", "text-align", "start"),
    ("text-end", "text-align", "end"),
    ("uppercase", "text-transform", "uppercase"),
    ("lowercase", "text-transform", "lowercase"),
    ("capitalize", "text-transform", "capitalize"),
    ("normal-case", "text-transform", "none"),
    ("whitespace-normal", "white-space", "normal"),
    ("whitespace-nowrap", "white-space", "nowrap"),
    ("whitespace-pre", "white-space", "pre"),
    ("whitespace-pre-line", "white-space", "pre-line"),
    ("whitespace-pre-wrap", "white-space", "pre-wrap"),
    ("whitespace-break-spaces", "white-space", "break-spaces"),
    ("italic", "font-style", "italic"),
    ("not-italic", "font-style", "normal"),
    ("cursor-auto", "cursor", "auto"),
    ("cursor-default", "cursor", "default"),
    ("cursor-pointer", "cursor", "pointer"),
    ("cursor-text", "cursor", "text"),
    ("cursor-move", "cursor", "move"),
    ("cursor-not-allowed", "cursor", "not-allowed"),
    ("cursor-grab", "cursor", "grab"),
    ("cursor-grabbing", "cursor", "grabbing"),
    ("pointer-events-none", "pointer-events", "none"),
    ("pointer-events-auto", "pointer-events", "auto"),
    ("box-border", "box-sizing", "border-box"),
    ("box-content", "box-sizing", "content-box"),
    ("object-contain", "object-fit", "contain"),
    ("object-cover", "object-fit", "cover"),
    ("object-fill", "object-fit", "fill"),
    ("object-none", "object-fit", "none"),
    ("object-scale-down", "object-fit", "scale-down"),
    ("isolate", "isolation", "isolate"),
    ("resize-none", "resize", "none"),
    ("text-ellipsis", "text-overflow", "ellipsis"),
    ("text-clip", "text-overflow", "clip"),
];

/// Spacing families: `(prefix, property)`. All take a scale step, `auto`/`px`-free literals
/// and negative steps where the property allows them.
const SPACING: &[(&str, &str, bool)] = &[
    ("p", "padding", false),
    ("pt", "padding-top", false),
    ("pr", "padding-right", false),
    ("pb", "padding-bottom", false),
    ("pl", "padding-left", false),
    ("m", "margin", true),
    ("mt", "margin-top", true),
    ("mr", "margin-right", true),
    ("mb", "margin-bottom", true),
    ("ml", "margin-left", true),
    ("gap", "gap", false),
    ("gap-x", "column-gap", false),
    ("gap-y", "row-gap", false),
    ("inset", "inset", true),
    ("top", "top", true),
    ("right", "right", true),
    ("bottom", "bottom", true),
    ("left", "left", true),
    ("w", "width", false),
    ("h", "height", false),
    ("min-w", "min-width", false),
    ("min-h", "min-height", false),
    ("max-w", "max-width", false),
    ("max-h", "max-height", false),
];

/// Sizing keywords: `(suffix, value)` per property family.
const SIZE_WORDS: &[(&str, &str)] = &[
    ("full", "100%"),
    ("fit", "fit-content"),
    ("min", "min-content"),
    ("max", "max-content"),
    ("auto", "auto"),
];

const COLOR_FAMILIES: &[(&str, &[&str])] = &[
    ("text", &["color"]),
    ("bg", &["background-color"]),
    ("border", &["border-color"]),
    ("border-t", &["border-top-color"]),
    ("border-r", &["border-right-color"]),
    ("border-b", &["border-bottom-color"]),
    ("border-l", &["border-left-color"]),
    ("outline", &["outline-color"]),
    ("fill", &["fill"]),
    ("stroke", &["stroke"]),
    ("caret", &["caret-color"]),
    ("accent", &["accent-color"]),
    ("decoration", &["text-decoration-color"]),
];

const RADIUS_FAMILIES: &[(&str, &str)] = &[
    ("rounded", "border-radius"),
    ("rounded-tl", "border-top-left-radius"),
    ("rounded-tr", "border-top-right-radius"),
    ("rounded-br", "border-bottom-right-radius"),
    ("rounded-bl", "border-bottom-left-radius"),
];

fn css_of(components: &[Component]) -> String {
    css::to_css(&css::trim(components))
}

fn number_text(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `calc(var(--spacing) * N)` / `var(--spacing)` -> N.
fn scale_step(value: &[Component]) -> Option<f64> {
    let v = css::trim(value);
    match v.as_slice() {
        [Component::Func(name, inner)] if name == "var" => {
            matches!(inner.as_slice(), [Component::Tok(Tok::Ident(n))] if n == "--spacing")
                .then_some(1.0)
        }
        [Component::Func(name, inner)] if name == "calc" => {
            let parts: Vec<&Component> = inner.iter().filter(|c| !c.is_whitespace()).collect();
            match parts.as_slice() {
                [
                    Component::Func(f, a),
                    Component::Tok(Tok::Delim('*')),
                    Component::Tok(Tok::Number { value: n, .. }),
                ] if f == "var"
                    && matches!(a.as_slice(), [Component::Tok(Tok::Ident(x))] if x == "--spacing") =>
                {
                    Some(f64::from(*n))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn var_name(value: &[Component]) -> Option<String> {
    match css::trim(value).as_slice() {
        [Component::Func(name, inner)] if name == "var" => match inner.as_slice() {
            [Component::Tok(Tok::Ident(n))] => Some(n.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Escape a CSS value for Tailwind's arbitrary-property syntax: spaces become `_`, literal
/// underscores `\_`. `None` when the value cannot be written safely inside a class token.
fn arbitrary_value(value: &str) -> Option<String> {
    // `]` would end the class; braces and semicolons would split the declaration; backslashes
    // and newlines are left to plain-CSS recipes; `&` would be decoded as an entity by JSX.
    if value.trim().is_empty() || value.contains(['\n', '\r', '\\', '[', ']', '{', '}', ';', '&']) {
        return None;
    }
    // Quotes become single quotes (JSX strings are double-quoted); that is only safe when the
    // value has no single quote of its own.
    if value.contains('"') && value.contains('\'') {
        return None;
    }
    // Tailwind does not decode `_` back to a space inside url(): leave those to plain CSS.
    let mut rest = value;
    while let Some(i) = rest.find("url(") {
        let after = &rest[i + 4..];
        let end = after.find(')').unwrap_or(after.len());
        if after[..end].contains(' ') {
            return None;
        }
        rest = &after[end..];
    }
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            ' ' => out.push('_'),
            '_' => out.push_str("\\_"),
            // JSX attribute strings are double-quoted: use single quotes inside values.
            '"' => out.push('\''),
            other => out.push(other),
        }
    }
    Some(out)
}

fn arbitrary(prop: &str, value: &str) -> Option<String> {
    Some(format!("[{prop}:{}]", arbitrary_value(value)?))
}

/// The utility (without variants) that stands for exactly `prop: value`, or `None` if even an
/// arbitrary property cannot carry the value (the caller then uses a plain-CSS recipe).
pub fn render(prop: &str, value: &[Component], theme: &Theme) -> Option<String> {
    let text = css_of(value);
    if let Some((class, _, _)) = KEYWORDS.iter().find(|(_, p, v)| *p == prop && *v == text) {
        return Some((*class).to_string());
    }
    if let Some(class) = spacing_class(prop, value, &text, theme) {
        return Some(class);
    }
    if let Some(name) = var_name(value) {
        if let Some(n) = name.strip_prefix("--color-")
            && theme.colors.contains(n)
            && !n.contains(['/', '.'])
        {
            for (prefix, props) in COLOR_FAMILIES {
                if props.len() == 1 && props[0] == prop {
                    // `text-<name>` is ambiguous when a font size has the same key.
                    if *prefix == "text" && theme.text_sizes.contains(n) {
                        break;
                    }
                    return Some(format!("{prefix}-{n}"));
                }
            }
        }
        if let Some(n) = name.strip_prefix("--radius-")
            && theme.radii.contains(n)
        {
            for (prefix, p) in RADIUS_FAMILIES {
                if *p == prop {
                    return Some(format!("{prefix}-{n}"));
                }
            }
        }
        if let Some(n) = name.strip_prefix("--font-")
            && prop == "font-family"
            && theme.fonts.contains(n)
        {
            return Some(format!("font-{n}"));
        }
    }
    if prop == "opacity"
        && let Ok(v) = text.parse::<f64>()
    {
        let pct = (v * 100.0).round();
        if (0.0..=100.0).contains(&pct) && (v * 100.0 - pct).abs() < 1e-9 {
            return Some(format!("opacity-{}", pct as i64));
        }
    }
    if prop == "z-index" {
        if text == "auto" {
            return Some("z-auto".into());
        }
        // Tailwind writes `-z-1` as `calc(1 * -1)`: not the declaration itself, so negatives stay arbitrary.
        if let Ok(n) = text.parse::<i64>()
            && n >= 0
        {
            return Some(format!("z-{n}"));
        }
    }
    arbitrary(prop, &text)
}

fn allows_auto(prefix: &str) -> bool {
    matches!(
        prefix,
        "m" | "mt"
            | "mr"
            | "mb"
            | "ml"
            | "inset"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "w"
            | "h"
            | "min-w"
            | "min-h"
    )
}

fn is_size(prefix: &str) -> bool {
    matches!(prefix, "w" | "h" | "min-w" | "min-h" | "max-w" | "max-h")
}

fn spacing_class(prop: &str, value: &[Component], text: &str, theme: &Theme) -> Option<String> {
    let (prefix, _, negatives) = SPACING.iter().find(|(_, p, _)| *p == prop)?;
    let free = |key: &str| !theme.shadowed_keys.contains(key);
    if (text == "0" || text == "0px") && free("0") {
        return Some(format!("{prefix}-0"));
    }
    if text == "auto" && allows_auto(prefix) && free("auto") {
        return Some(format!("{prefix}-auto"));
    }
    if is_size(prefix) {
        for (word, v) in SIZE_WORDS {
            if *word != "auto" && *v == text && free(word) {
                return Some(format!("{prefix}-{word}"));
            }
        }
        if text == "100vw" && matches!(*prefix, "w" | "min-w" | "max-w") && free("screen") {
            return Some(format!("{prefix}-screen"));
        }
        if text == "100vh" && matches!(*prefix, "h" | "min-h" | "max-h") && free("screen") {
            return Some(format!("{prefix}-screen"));
        }
        if text == "none" && matches!(*prefix, "max-w" | "max-h") && free("none") {
            return Some(format!("{prefix}-none"));
        }
    }
    theme.spacing_base_px?;
    let n = scale_step(value)?;
    if n < 0.0 && !negatives {
        return None;
    }
    // Steps are multiples of one half; anything else would not be a valid utility.
    if (n * 2.0 - (n * 2.0).round()).abs() > 1e-9 || n == 0.0 {
        return None;
    }
    let abs = number_text(n.abs());
    if !free(&abs) {
        return None;
    }
    Some(if n < 0.0 {
        format!("-{prefix}-{abs}")
    } else {
        format!("{prefix}-{abs}")
    })
}

/// Declarations a utility stands for (variants already removed). `None` = not a utility this
/// module renders.
pub fn expand(class: &str, theme: &Theme) -> Option<Vec<(String, String)>> {
    if let Some((_, p, v)) = KEYWORDS.iter().find(|(c, _, _)| *c == class) {
        return Some(vec![((*p).to_string(), (*v).to_string())]);
    }
    if let Some(inner) = class.strip_prefix('[').and_then(|c| c.strip_suffix(']')) {
        let (prop, value) = inner.split_once(':')?;
        return Some(vec![(prop.to_string(), unescape_arbitrary(value))]);
    }
    let (negative, body) = match class.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, class),
    };
    if let Some(n) = body.strip_prefix("z-") {
        if n == "auto" && !negative {
            return Some(vec![("z-index".into(), "auto".into())]);
        }
        return n.parse::<i64>().ok().map(|v| {
            let value = if negative {
                format!("calc({v} * -1)")
            } else {
                v.to_string()
            };
            vec![("z-index".to_string(), value)]
        });
    }
    if let Some(n) = class.strip_prefix("opacity-") {
        return n
            .parse::<u32>()
            .ok()
            .filter(|v| *v <= 100)
            .map(|v| vec![("opacity".to_string(), format!("{v}%"))]);
    }
    for (prefix, prop, negatives) in SPACING {
        if let Some(rest) = body.strip_prefix(&format!("{prefix}-")) {
            if negative && !negatives {
                continue;
            }
            // Longest prefix wins (`gap-x` before `gap`, `min-w` before `m`): the table order
            // lists the longer families first where they would clash, so check the rest parses.
            if theme.shadowed_keys.contains(rest) {
                return None; // the theme redefines this key: not the built-in meaning
            }
            if rest == "0" {
                return Some(vec![((*prop).to_string(), "0px".to_string())]);
            }
            if rest == "auto" && !negative && allows_auto(prefix) {
                return Some(vec![((*prop).to_string(), "auto".to_string())]);
            }
            if is_size(prefix) && !negative {
                if let Some((_, v)) = SIZE_WORDS.iter().find(|(w, _)| *w == rest) {
                    return Some(vec![((*prop).to_string(), (*v).to_string())]);
                }
                if rest == "screen" {
                    let v = if matches!(*prefix, "h" | "min-h" | "max-h") {
                        "100vh"
                    } else {
                        "100vw"
                    };
                    return Some(vec![((*prop).to_string(), v.to_string())]);
                }
                if rest == "none" && matches!(*prefix, "max-w" | "max-h") {
                    return Some(vec![((*prop).to_string(), "none".to_string())]);
                }
            }
            if let Ok(n) = rest.parse::<f64>() {
                theme.spacing_base_px?;
                let n = if negative { -n } else { n };
                let value = if (n - 1.0).abs() < 1e-9 {
                    "var(--spacing)".to_string()
                } else {
                    format!("calc(var(--spacing) * {})", number_text(n))
                };
                return Some(vec![((*prop).to_string(), value)]);
            }
        }
    }
    for (prefix, props) in COLOR_FAMILIES {
        if let Some(name) = class.strip_prefix(&format!("{prefix}-"))
            && theme.colors.contains(name)
        {
            return Some(
                props
                    .iter()
                    .map(|p| ((*p).to_string(), format!("var(--color-{name})")))
                    .collect(),
            );
        }
    }
    for (prefix, prop) in RADIUS_FAMILIES {
        if let Some(name) = class.strip_prefix(&format!("{prefix}-"))
            && theme.radii.contains(name)
        {
            return Some(vec![((*prop).to_string(), format!("var(--radius-{name})"))]);
        }
    }
    if let Some(name) = class.strip_prefix("font-")
        && theme.fonts.contains(name)
    {
        return Some(vec![("font-family".into(), format!("var(--font-{name})"))]);
    }
    None
}

pub fn unescape_arbitrary(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'_') => {
                chars.next();
                out.push('_');
            }
            '_' => out.push(' '),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::css::parse_components;
    use super::*;

    fn theme() -> Theme {
        let names: Vec<String> = [
            "--color-ink",
            "--color-white",
            "--radius-card",
            "--font-sans",
            "--text-sm",
            "--color-sm",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        Theme::from_definitions(names.iter(), Some(4.0))
    }

    fn render_text(prop: &str, value: &str) -> String {
        render(prop, &parse_components(value), &theme()).unwrap()
    }

    #[test]
    fn plain_utilities_expand_to_exactly_the_declaration() {
        for (prop, value, class) in [
            ("display", "flex", "flex"),
            ("display", "none", "hidden"),
            ("align-items", "center", "items-center"),
            ("padding-top", "calc(var(--spacing) * 4)", "pt-4"),
            ("padding", "var(--spacing)", "p-1"),
            ("margin-left", "calc(var(--spacing) * -2.5)", "-ml-2.5"),
            ("gap", "0", "gap-0"),
            ("width", "100%", "w-full"),
            ("height", "100vh", "h-screen"),
            ("max-width", "none", "max-w-none"),
            ("margin-left", "auto", "ml-auto"),
            ("color", "var(--color-ink)", "text-ink"),
            ("background-color", "var(--color-white)", "bg-white"),
            ("border-radius", "var(--radius-card)", "rounded-card"),
            ("font-family", "var(--font-sans)", "font-sans"),
            ("opacity", "0.37", "opacity-37"),
        ] {
            assert_eq!(render_text(prop, value), class, "{prop}: {value}");
            let expanded =
                expand(class, &theme()).unwrap_or_else(|| panic!("{class} has no expansion"));
            assert_eq!(expanded.len(), 1, "{class}");
            assert_eq!(expanded[0].0, prop, "{class}");
        }
    }

    #[test]
    fn ambiguous_or_extra_declaration_utilities_become_arbitrary_properties() {
        // `text-sm` is also a font size here, so a color with that key must not use it.
        assert_eq!(
            render_text("color", "var(--color-sm)"),
            "[color:var(--color-sm)]"
        );
        assert_eq!(render_text("font-weight", "500"), "[font-weight:500]");
        assert_eq!(
            render_text("padding-inline", "calc(var(--spacing) * 4)"),
            "[padding-inline:calc(var(--spacing)_*_4)]"
        );
        assert_eq!(render_text("padding-top", "7px"), "[padding-top:7px]");
        assert_eq!(
            render_text("z-index", "-3"),
            "[z-index:-3]",
            "-z-3 is calc(3 * -1) in Tailwind"
        );
        assert_eq!(
            render_text("box-shadow", "0 1px 2px #000"),
            "[box-shadow:0_1px_2px_#000]"
        );
        assert_eq!(
            render_text("padding-top", "calc(var(--spacing) * 0.3)"),
            "[padding-top:calc(var(--spacing)_*_0.3)]"
        );
    }

    #[test]
    fn arbitrary_values_are_escaped_and_round_trip() {
        let class = render_text("font-family", "\"STK Gerhard\", sans-serif");
        assert_eq!(class, "[font-family:'STK_Gerhard',_sans-serif]");
        let back = expand(&class, &theme()).unwrap();
        assert_eq!(
            back[0],
            (
                "font-family".to_string(),
                "'STK Gerhard', sans-serif".to_string()
            )
        );
        let class = render_text("content", "\"a_b\"");
        assert_eq!(class, "[content:'a\\_b']");
        assert_eq!(expand(&class, &theme()).unwrap()[0].1, "'a_b'");
        assert!(
            render(
                "background",
                &parse_components("url(\"a]b.png\")"),
                &theme()
            )
            .is_none(),
            "`]` cannot live in a class token"
        );
    }

    #[test]
    fn theme_keys_that_redefine_builtin_sizes_force_arbitrary_properties() {
        let names = [
            "--spacing-0".to_string(),
            "--spacing-full".to_string(),
            "--container-4".to_string(),
        ];
        let theme = Theme::from_definitions(names.iter(), Some(4.0));
        assert_eq!(
            render("padding-top", &parse_components("0"), &theme).unwrap(),
            "[padding-top:0]"
        );
        assert_eq!(
            render("height", &parse_components("100%"), &theme).unwrap(),
            "[height:100%]"
        );
        assert_eq!(
            render(
                "margin-top",
                &parse_components("calc(var(--spacing) * 4)"),
                &theme
            )
            .unwrap(),
            "[margin-top:calc(var(--spacing)_*_4)]"
        );
        assert_eq!(
            render(
                "margin-top",
                &parse_components("calc(var(--spacing) * 5)"),
                &theme
            )
            .unwrap(),
            "mt-5"
        );
        assert!(expand("h-full", &theme).is_none());
    }

    #[test]
    fn a_scale_is_required_for_spacing_utilities() {
        let no_scale = Theme::default();
        assert!(
            render(
                "padding-top",
                &parse_components("calc(var(--spacing) * 4)"),
                &no_scale
            )
            .unwrap()
            .starts_with('[')
        );
        assert!(expand("pt-4", &no_scale).is_none());
    }
}
