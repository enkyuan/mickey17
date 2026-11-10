//! Typed reading of declaration values and unit-normalized comparison keys.
//!
//! A *key* is what two values must share to be merged into one token. Keys are only ever
//! equal when the browser would compute the same value:
//! * sRGB colors compare by 8-bit channels and 8-bit alpha (`#fff`, `white`,
//!   `rgb(255 255 255 / 100%)` share a key); other color spaces compare by their own
//!   numbers and never merge with sRGB.
//! * absolute lengths compare in CSS pixels; `rem` joins them only when the root font
//!   size is known to be constant ([`Context::rem_px`]); `em`, `%` and viewport units
//!   compare only with their own unit.
//! * times compare in milliseconds, numbers to four decimals.

use super::css::{Component, Tok, to_css};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Color,
    Spacing,
    Size,
    Radius,
    FontFamily,
    FontSize,
    FontWeight,
    LetterSpacing,
    LineHeight,
    Shadow,
    BorderWidth,
    Easing,
    Duration,
    ZIndex,
    Opacity,
    Breakpoint,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Color => "color",
            Category::Spacing => "spacing",
            Category::Size => "size",
            Category::Radius => "radius",
            Category::FontFamily => "font-family",
            Category::FontSize => "font-size",
            Category::FontWeight => "font-weight",
            Category::LetterSpacing => "tracking",
            Category::LineHeight => "leading",
            Category::Shadow => "shadow",
            Category::BorderWidth => "border-width",
            Category::Easing => "ease",
            Category::Duration => "duration",
            Category::ZIndex => "z-index",
            Category::Opacity => "opacity",
            Category::Breakpoint => "breakpoint",
        }
    }
}

/// Values needed to normalize relative units.
#[derive(Debug, Clone, Copy)]
pub struct Context {
    /// CSS pixels per `rem`, only when it is the same in every captured snapshot.
    pub rem_px: Option<f64>,
}

fn round4(v: f64) -> f64 {
    let r = (v * 10_000.0).round() / 10_000.0;
    if r == 0.0 { 0.0 } else { r }
}

pub fn fmt_num(v: f64) -> String {
    let r = round4(v);
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

// ----- colors --------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    /// 8-bit sRGB channels and 8-bit alpha.
    Srgb([u8; 4]),
    /// Any other color syntax, compared by a canonical text key.
    Other(String),
}

impl Color {
    pub fn key(&self) -> String {
        match self {
            Color::Srgb([r, g, b, a]) => format!("srgb:{r},{g},{b},{a}"),
            Color::Other(text) => format!("other:{text}"),
        }
    }
    pub fn srgb(&self) -> Option<[u8; 4]> {
        match self {
            Color::Srgb(c) => Some(*c),
            Color::Other(_) => None,
        }
    }
}

fn clamp8(v: f64) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

fn number_of(c: &Component) -> Option<f64> {
    match c {
        Component::Tok(Tok::Number { value, .. }) => Some(f64::from(*value)),
        _ => None,
    }
}

fn args_of(components: &[Component]) -> Vec<&Component> {
    components.iter().filter(|c| !c.is_whitespace()).collect()
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    let h = h.rem_euclid(360.0) / 360.0;
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |mut t: f64| {
        t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        v * 255.0
    };
    [channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)]
}

/// One channel of a modern/legacy color function: number, percentage, or angle for hue.
fn channel(c: &Component, percent_scale: f64) -> Option<f64> {
    match c {
        Component::Tok(Tok::Number { value, .. }) => Some(f64::from(*value)),
        Component::Tok(Tok::Percentage { unit_value, .. }) => {
            Some(f64::from(*unit_value) * percent_scale)
        }
        _ => None,
    }
}

fn hue(c: &Component) -> Option<f64> {
    match c {
        Component::Tok(Tok::Number { value, .. }) => Some(f64::from(*value)),
        Component::Tok(Tok::Dimension { value, unit, .. }) => {
            let v = f64::from(*value);
            match unit.to_ascii_lowercase().as_str() {
                "deg" => Some(v),
                "grad" => Some(v * 0.9),
                "rad" => Some(v.to_degrees()),
                "turn" => Some(v * 360.0),
                _ => None,
            }
        }
        _ => None,
    }
}

fn alpha_of(parts: &[&Component]) -> Option<u8> {
    match parts {
        [] => Some(255),
        [c] => channel(c, 1.0).map(|a| clamp8(a.clamp(0.0, 1.0) * 255.0)),
        _ => None,
    }
}

/// Split color-function arguments into (components, alpha) for both comma and slash syntax.
fn split_color_args<'a>(args: &[&'a Component]) -> (Vec<&'a Component>, Vec<&'a Component>) {
    let mut main = Vec::new();
    let mut alpha = Vec::new();
    let mut in_alpha = false;
    for c in args {
        match c {
            Component::Tok(Tok::Delim('/')) => in_alpha = true,
            Component::Tok(Tok::Comma) => {
                // Legacy syntax: the fourth argument is the alpha.
                if main.len() == 3 {
                    in_alpha = true;
                }
            }
            other => {
                if in_alpha {
                    alpha.push(*other)
                } else {
                    main.push(*other)
                }
            }
        }
    }
    (main, alpha)
}

fn named(name: &str) -> Option<[u8; 4]> {
    Some(match name {
        "transparent" => [0, 0, 0, 0],
        "black" => [0, 0, 0, 255],
        "white" => [255, 255, 255, 255],
        _ => return None,
    })
}

fn hex(text: &str) -> Option<[u8; 4]> {
    let digit = |c: char| c.to_digit(16).map(|d| d as u8);
    let chars: Vec<u8> = text.chars().map(digit).collect::<Option<_>>()?;
    match chars.len() {
        3 => Some([chars[0] * 17, chars[1] * 17, chars[2] * 17, 255]),
        4 => Some([chars[0] * 17, chars[1] * 17, chars[2] * 17, chars[3] * 17]),
        6 => Some([
            chars[0] * 16 + chars[1],
            chars[2] * 16 + chars[3],
            chars[4] * 16 + chars[5],
            255,
        ]),
        8 => Some([
            chars[0] * 16 + chars[1],
            chars[2] * 16 + chars[3],
            chars[4] * 16 + chars[5],
            chars[6] * 16 + chars[7],
        ]),
        _ => None,
    }
}

fn canonical_other(name: &str, args: &[Component]) -> String {
    // Numbers are re-printed so `.5` and `0.50` agree; everything else is kept verbatim.
    fn walk(components: &[Component], out: &mut String) {
        for (index, c) in components.iter().enumerate() {
            match c {
                Component::Tok(Tok::Space)
                    if out.ends_with(',')
                        || components.get(index + 1).is_some_and(Component::is_comma) => {}
                Component::Tok(Tok::Number { value, .. }) => {
                    out.push_str(&fmt_num(f64::from(*value)))
                }
                Component::Tok(Tok::Percentage { unit_value, .. }) => {
                    out.push_str(&fmt_num(f64::from(*unit_value) * 100.0));
                    out.push('%');
                }
                Component::Tok(Tok::Dimension { value, unit, .. }) => {
                    out.push_str(&fmt_num(f64::from(*value)));
                    out.push_str(&unit.to_ascii_lowercase());
                }
                Component::Tok(Tok::Ident(i)) => out.push_str(&i.to_ascii_lowercase()),
                Component::Func(n, inner) => {
                    out.push_str(&n.to_ascii_lowercase());
                    out.push('(');
                    walk(inner, out);
                    out.push(')');
                }
                other => other.write(out),
            }
        }
    }
    let mut out = format!("{}(", name.to_ascii_lowercase());
    walk(&super::css::trim(args), &mut out);
    out.push(')');
    out
}

/// Read a color from one component (`#fff`, `rgb(...)`, `oklch(...)`, `transparent`...).
pub fn parse_color(component: &Component) -> Option<Color> {
    match component {
        Component::Tok(Tok::Hash(h) | Tok::IdHash(h)) => hex(h).map(Color::Srgb),
        Component::Tok(Tok::Ident(name)) => named(&name.to_ascii_lowercase()).map(Color::Srgb),
        Component::Func(name, inner) => {
            let lower = name.to_ascii_lowercase();
            let args = args_of(inner);
            let (main, alpha) = split_color_args(&args);
            match lower.as_str() {
                "rgb" | "rgba" => {
                    if main.len() != 3
                        || main
                            .iter()
                            .any(|c| matches!(c, Component::Tok(Tok::Ident(_))))
                    {
                        return Some(Color::Other(canonical_other(&lower, inner)));
                    }
                    let r = channel(main[0], 255.0)?;
                    let g = channel(main[1], 255.0)?;
                    let b = channel(main[2], 255.0)?;
                    let a = alpha_of(&alpha)?;
                    Some(Color::Srgb([clamp8(r), clamp8(g), clamp8(b), a]))
                }
                "hsl" | "hsla" => {
                    if main.len() != 3 {
                        return Some(Color::Other(canonical_other(&lower, inner)));
                    }
                    let h = hue(main[0])?;
                    let s = channel(main[1], 1.0)?;
                    let l = channel(main[2], 1.0)?;
                    // Numbers without % are percentages in hsl().
                    let s = if matches!(main[1], Component::Tok(Tok::Number { .. })) {
                        s / 100.0
                    } else {
                        s
                    };
                    let l = if matches!(main[2], Component::Tok(Tok::Number { .. })) {
                        l / 100.0
                    } else {
                        l
                    };
                    let [r, g, b] = hsl_to_rgb(h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
                    let a = alpha_of(&alpha)?;
                    Some(Color::Srgb([clamp8(r), clamp8(g), clamp8(b), a]))
                }
                "oklch" | "oklab" | "lab" | "lch" | "hwb" | "color" | "color-mix"
                | "light-dark" => Some(Color::Other(canonical_other(&lower, inner))),
                _ => None,
            }
        }
        _ => None,
    }
}

pub fn color_css(color: &Color, original: &Component) -> String {
    match color {
        Color::Srgb(_) => to_css(std::slice::from_ref(original)),
        Color::Other(_) => to_css(std::slice::from_ref(original)),
    }
}

// ----- lengths, times, numbers ---------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Length {
    /// Absolute length in CSS pixels (or rem converted through a constant root size).
    Px(f64),
    /// Anything unit-relative: key keeps the unit.
    Relative(f64, String),
}

impl Length {
    pub fn key(&self) -> String {
        match self {
            Length::Px(v) => format!("px:{}", fmt_num(*v)),
            Length::Relative(v, unit) => format!("{unit}:{}", fmt_num(*v)),
        }
    }
    pub fn px(&self) -> Option<f64> {
        match self {
            Length::Px(v) => Some(*v),
            Length::Relative(..) => None,
        }
    }
}

pub fn parse_length(component: &Component, ctx: &Context) -> Option<Length> {
    match component {
        Component::Tok(Tok::Dimension { value, unit, .. }) => {
            let v = f64::from(*value);
            let unit = unit.to_ascii_lowercase();
            Some(match unit.as_str() {
                "px" => Length::Px(v),
                "pt" => Length::Px(v * 96.0 / 72.0),
                "pc" => Length::Px(v * 16.0),
                "in" => Length::Px(v * 96.0),
                "cm" => Length::Px(v * 96.0 / 2.54),
                "mm" => Length::Px(v * 96.0 / 25.4),
                "q" => Length::Px(v * 96.0 / 101.6),
                "rem" => match ctx.rem_px {
                    Some(root) => Length::Px(v * root),
                    None => Length::Relative(v, "rem".into()),
                },
                "em" | "ex" | "ch" | "lh" | "rlh" | "vw" | "vh" | "vmin" | "vmax" | "svw"
                | "svh" | "lvw" | "lvh" | "dvw" | "dvh" | "cqw" | "cqh" | "cqi" | "cqb" | "cap"
                | "ic" => Length::Relative(v, unit),
                _ => return None,
            })
        }
        Component::Tok(Tok::Percentage { unit_value, .. }) => {
            Some(Length::Relative(f64::from(*unit_value) * 100.0, "%".into()))
        }
        _ => None,
    }
}

/// Time in milliseconds.
pub fn parse_time(component: &Component) -> Option<f64> {
    match component {
        Component::Tok(Tok::Dimension { value, unit, .. }) => {
            match unit.to_ascii_lowercase().as_str() {
                "ms" => Some(f64::from(*value)),
                "s" => Some(f64::from(*value) * 1000.0),
                _ => None,
            }
        }
        _ => None,
    }
}

// ----- composite values ----------------------------------------------------

/// Split a component list at top-level commas.
pub fn split_commas(components: &[Component]) -> Vec<Vec<Component>> {
    let mut out = vec![Vec::new()];
    for c in components {
        if c.is_comma() {
            out.push(Vec::new());
        } else {
            out.last_mut().expect("never empty").push(c.clone());
        }
    }
    out.into_iter()
        .map(|part| super::css::trim(&part))
        .collect()
}

/// Split at the first top-level comma only (`var(--x, a, b)` keeps `a, b` as the fallback).
pub fn split_commas_first(components: &[Component]) -> (Vec<Component>, Option<Vec<Component>>) {
    match components.iter().position(Component::is_comma) {
        Some(i) => (
            components[..i].to_vec(),
            Some(super::css::trim(&components[i + 1..])),
        ),
        None => (components.to_vec(), None),
    }
}

const GENERIC_FAMILIES: &[&str] = &[
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
    "math",
    "emoji",
    "fangsong",
];

pub fn font_family_key(components: &[Component]) -> Option<String> {
    // CSS-wide keywords are not families: `var(--x)` of `initial` is guaranteed-invalid.
    if let [Component::Tok(Tok::Ident(i))] = components
        && matches!(
            i.to_ascii_lowercase().as_str(),
            "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default"
        )
    {
        return None;
    }
    let mut names = Vec::new();
    for part in split_commas(components) {
        let mut words = Vec::new();
        for c in &part {
            match c {
                Component::Tok(Tok::Str(s)) => {
                    // A quoted generic name is a real family called "serif", not the generic.
                    let lower = s.to_ascii_lowercase();
                    if GENERIC_FAMILIES.contains(&lower.as_str()) {
                        words.push(format!("\"{lower}\""));
                    } else {
                        words.push(lower);
                    }
                }
                Component::Tok(Tok::Ident(i)) => words.push(i.to_ascii_lowercase()),
                Component::Tok(Tok::Space) => {}
                Component::Func(..) => return None, // var(), env(): not a literal list
                _ => return None,
            }
        }
        if words.is_empty() {
            return None;
        }
        names.push(words.join(" "));
    }
    Some(names.join(","))
}

pub fn font_weight_key(components: &[Component]) -> Option<String> {
    match components {
        [Component::Tok(Tok::Number { value, .. })] => Some(fmt_num(f64::from(*value))),
        [Component::Tok(Tok::Ident(i))] => match i.to_ascii_lowercase().as_str() {
            "normal" => Some("400".into()),
            "bold" => Some("700".into()),
            _ => None,
        },
        _ => None,
    }
}

fn is_keyword(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "none"
    )
}

/// One shadow layer split into its parts.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowLayer {
    pub inset: bool,
    pub lengths: Vec<Length>,
    pub color: Option<Color>,
    pub color_component: Option<Component>,
}

/// `text` selects `text-shadow` syntax (2-3 lengths, no `inset`); otherwise `box-shadow` (2-4).
pub fn parse_shadow(
    components: &[Component],
    ctx: &Context,
    text: bool,
) -> Option<Vec<ShadowLayer>> {
    if matches!(components, [Component::Tok(Tok::Ident(i))] if is_keyword(i)) {
        return None;
    }
    let mut layers = Vec::new();
    for part in split_commas(components) {
        let mut layer = ShadowLayer {
            inset: false,
            lengths: Vec::new(),
            color: None,
            color_component: None,
        };
        for c in part.iter().filter(|c| !c.is_whitespace()) {
            if c.ident().is_some_and(|i| i.eq_ignore_ascii_case("inset")) {
                layer.inset = true;
            } else if let Some(color) = parse_color(c) {
                if layer.color.is_some() {
                    return None;
                }
                layer.color = Some(color);
                layer.color_component = Some(c.clone());
            } else if let Some(len) = parse_length(c, ctx) {
                layer.lengths.push(len);
            } else if matches!(c, Component::Tok(Tok::Number { value, .. }) if *value == 0.0) {
                layer.lengths.push(Length::Px(0.0));
            } else {
                return None;
            }
        }
        let max = if text { 3 } else { 4 };
        if !(2..=max).contains(&layer.lengths.len()) || (text && layer.inset) {
            return None;
        }
        layers.push(layer);
    }
    Some(layers)
}

/// The key keeps box- and text-shadows apart: `text-shadow` has no spread, so a box-shadow
/// token must never be offered to it.
pub fn shadow_key(layers: &[ShadowLayer], text: bool) -> String {
    let body = layers
        .iter()
        .map(|l| {
            let mut lengths: Vec<String> = l.lengths.iter().map(Length::key).collect();
            while lengths.len() < 4 {
                lengths.push("px:0".into());
            }
            let color = l
                .color
                .as_ref()
                .map_or("currentcolor".to_string(), Color::key);
            format!(
                "{}{} {}",
                if l.inset { "inset " } else { "" },
                lengths.join(" "),
                color
            )
        })
        .collect::<Vec<_>>()
        .join(" | ");
    format!("{}:{body}", if text { "text" } else { "box" })
}

pub fn easing_key(component: &Component) -> Option<String> {
    match component {
        Component::Func(name, inner) => {
            let lower = name.to_ascii_lowercase();
            match lower.as_str() {
                "cubic-bezier" | "steps" | "linear" => Some(canonical_other(&lower, inner)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Integer value of a lone number component.
pub fn parse_int(components: &[Component]) -> Option<i64> {
    match components {
        [
            Component::Tok(Tok::Number {
                int_value: Some(v), ..
            }),
        ] => Some(i64::from(*v)),
        _ => None,
    }
}

pub fn parse_number(components: &[Component]) -> Option<f64> {
    match components {
        [c] => number_of(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::css::parse_components;
    use super::*;

    fn color(text: &str) -> Option<Color> {
        let c = parse_components(text);
        parse_color(c.iter().find(|c| !c.is_whitespace())?)
    }

    fn len(text: &str, rem: Option<f64>) -> Option<Length> {
        let c = parse_components(text);
        parse_length(
            c.iter().find(|c| !c.is_whitespace())?,
            &Context { rem_px: rem },
        )
    }

    #[test]
    fn equal_srgb_spellings_share_a_key() {
        let reference = color("#ffffff").unwrap().key();
        for s in [
            "#fff",
            "white",
            "rgb(255 255 255)",
            "rgba(255, 255, 255, 1)",
            "rgb(100% 100% 100% / 100%)",
            "hsl(0 0% 100%)",
            "hsl(0, 0%, 100%)",
        ] {
            assert_eq!(color(s).unwrap().key(), reference, "{s}");
        }
        assert_eq!(
            color("#0000").unwrap().key(),
            color("transparent").unwrap().key()
        );
        assert_eq!(
            color("#00000080").unwrap().key(),
            color("rgb(0 0 0 / 50%)").unwrap().key()
        );
        assert_ne!(color("#fff").unwrap().key(), color("#fffe").unwrap().key());
    }

    #[test]
    fn other_color_spaces_never_merge_with_srgb() {
        let a = color("oklch(94.3% .016 250)").unwrap();
        let b = color("oklch(94.3% 0.0160 250)").unwrap();
        assert_eq!(a.key(), b.key());
        assert!(a.srgb().is_none());
        assert_ne!(a.key(), color("#eef").unwrap().key());
        assert!(color("red").is_none(), "unlisted named colors stay literal");
    }

    #[test]
    fn lengths_normalize_units_only_when_safe() {
        assert_eq!(
            len("1rem", Some(16.0)).unwrap().key(),
            len("16px", Some(16.0)).unwrap().key()
        );
        assert_eq!(
            len("12pt", None).unwrap().key(),
            len("16px", None).unwrap().key()
        );
        assert_ne!(
            len("1rem", None).unwrap().key(),
            len("16px", None).unwrap().key()
        );
        assert_ne!(
            len("1em", Some(16.0)).unwrap().key(),
            len("16px", Some(16.0)).unwrap().key()
        );
        assert_eq!(len("0.5rem", Some(16.0)).unwrap().px(), Some(8.0));
        assert_eq!(len("50%", None).unwrap().key(), "%:50");
    }

    #[test]
    fn shadows_and_easings_have_stable_keys() {
        let ctx = Context { rem_px: Some(16.0) };
        let a = parse_shadow(
            &parse_components("0 1px 2px rgba(0,0,0,.5), inset 0 0 0 1px #fff"),
            &ctx,
            false,
        )
        .unwrap();
        let b = parse_shadow(
            &parse_components("0px 0.0625rem 2px rgb(0 0 0 / 50%),inset 0 0 0 1px white"),
            &ctx,
            false,
        )
        .unwrap();
        assert_eq!(shadow_key(&a, false), shadow_key(&b, false));
        let text = parse_shadow(&parse_components("0 1px 2px #000"), &ctx, true).unwrap();
        let boxed = parse_shadow(&parse_components("0 1px 2px #000"), &ctx, false).unwrap();
        assert_ne!(
            shadow_key(&text, true),
            shadow_key(&boxed, false),
            "text and box shadows never share a token"
        );
        assert!(
            parse_shadow(&parse_components("0 1px 2px 0 #000"), &ctx, true).is_none(),
            "text-shadow has no spread"
        );
        assert!(parse_shadow(&parse_components("none"), &ctx, false).is_none());
        let e1 = parse_components("cubic-bezier(.4, 0, .2, 1)");
        let e2 = parse_components("cubic-bezier(0.4,0,0.2,1)");
        assert_eq!(easing_key(&e1[0]), easing_key(&e2[0]));
    }

    #[test]
    fn quoted_generic_family_names_are_not_the_generic() {
        let quoted = parse_components("\"serif\"");
        let generic = parse_components("serif");
        assert_ne!(font_family_key(&quoted), font_family_key(&generic));
    }

    #[test]
    fn font_families_ignore_quoting_and_case() {
        let a = parse_components("\"STK Gerhard\", Arial, sans-serif");
        let b = parse_components("'stk gerhard',arial,SANS-SERIF");
        assert_eq!(font_family_key(&a), font_family_key(&b));
        assert!(font_family_key(&parse_components("var(--x), serif")).is_none());
        assert_eq!(font_weight_key(&parse_components("bold")).unwrap(), "700");
        for keyword in ["initial", "inherit", "unset", "revert", "revert-layer"] {
            assert!(
                font_family_key(&parse_components(keyword)).is_none(),
                "{keyword}"
            );
        }
    }
}
