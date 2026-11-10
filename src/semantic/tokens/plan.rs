//! Token decisions: which values become tokens, what they are called and why, which
//! values stay literal, and which site custom properties move into the central file.

use super::css::{self, Component, Tok};
use super::inventory::{Group, Inventory, SiteVar, classify_var};
use super::recognize::Hit;
use super::value::{Category, Context, Length, fmt_num};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone)]
pub struct Options {
    /// A value must appear in at least this many declarations to get a new token.
    pub min_uses: usize,
    /// Merge px and rem spellings of the same computed length (computed-equal at the
    /// constant root font size; differs only if the user changes the browser font size).
    pub merge_rem_px: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            min_uses: 2,
            merge_rem_px: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Named by the site itself (custom property, kept verbatim).
    Site,
    /// Named by this pipeline.
    Derived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    SiteName,
    PageRole,
    UsageSubject,
    TailwindDefault,
    Palette,
    Value,
}

impl Basis {
    pub fn label(self) -> &'static str {
        match self {
            Basis::SiteName => "site name",
            Basis::PageRole => "page role",
            Basis::UsageSubject => "usage subject",
            Basis::TailwindDefault => "standard scale",
            Basis::Palette => "palette (hue + lightness)",
            Basis::Value => "value",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Token {
    pub name: String,
    pub category: Category,
    #[serde(skip)]
    pub key: String,
    /// Literal CSS emitted for the token.
    pub value: String,
    pub origin: Origin,
    pub basis: Basis,
    /// Declarations that contained the value in the source stylesheets.
    pub uses: usize,
    pub weight: u64,
    pub properties: Vec<String>,
    /// Distinct source spellings merged into this token.
    pub merged_spellings: Vec<String>,
    /// px and rem spellings were merged (computed-equal only at the captured root font size).
    pub merged_across_units: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Unnamed {
    pub category: Category,
    pub value: String,
    pub uses: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpacingScale {
    pub name: String,
    pub base_px: f64,
    /// Literal emitted for the base (`0.25rem` or `4px`).
    pub value: String,
    pub site_defined: bool,
    pub members: usize,
    pub off_scale: usize,
    /// Declarations written in px that now use a rem scale (or the reverse). Equal at the
    /// captured root font size, not under a different browser font size.
    pub converted_between_px_and_rem: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hoisted {
    pub name: String,
    pub value: String,
    pub category: Option<Category>,
    pub references: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeptVar {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Breakpoint {
    pub name: String,
    pub px: f64,
    pub uses: usize,
    pub basis: Basis,
}

#[derive(Debug, Default)]
pub struct TokenSet {
    pub tokens: Vec<Token>,
    by_key: HashMap<(Category, String), usize>,
    pub spacing: Option<SpacingScale>,
    pub hoisted: Vec<Hoisted>,
    pub kept_vars: Vec<KeptVar>,
    pub unnamed: Vec<Unnamed>,
    pub single_use: BTreeMap<Category, usize>,
    pub breakpoints: Vec<Breakpoint>,
    pub ctx: Option<Context>,
    /// Decisions that limit what the output claims; copied into the report's gap list.
    pub gaps: Vec<String>,
}

const INTERNAL_PREFIXES: &[&str] = &[
    "--tw-",
    "--_",
    "--radix-",
    "--base-",
    "--framer-",
    "--motion-",
    "--webkit-",
];

/// Does the group contain length spellings in a different unit (px vs rem) than the variable's?
fn crosses_units(group: &Group, var_value: &[Component]) -> bool {
    let var_unit = var_value.iter().find_map(|c| match c {
        Component::Tok(Tok::Dimension { unit, .. }) => Some(unit.to_ascii_lowercase()),
        _ => None,
    });
    let mixed = group.units.contains_key("px") && group.units.contains_key("rem");
    match var_unit.as_deref() {
        Some(u @ ("px" | "rem")) => {
            mixed
                || group
                    .units
                    .keys()
                    .any(|g| matches!(g.as_str(), "px" | "rem") && g != u)
        }
        _ => mixed,
    }
}

fn signature(value: &[Component]) -> String {
    css::to_css(&css::trim(value))
}

fn references_vars(value: &[Component], out: &mut Vec<String>) {
    for c in value {
        match c {
            Component::Func(name, inner) => {
                if name.eq_ignore_ascii_case("var")
                    && let Some(Component::Tok(Tok::Ident(n))) =
                        inner.iter().find(|c| !c.is_whitespace())
                {
                    out.push(n.clone());
                }
                references_vars(inner, out);
            }
            Component::Paren(inner) | Component::Square(inner) | Component::Curly(inner) => {
                references_vars(inner, out)
            }
            _ => {}
        }
    }
}

fn has_env_or_attr(value: &[Component]) -> bool {
    value.iter().any(|c| match c {
        Component::Func(name, inner) => {
            matches!(name.to_ascii_lowercase().as_str(), "env" | "attr") || has_env_or_attr(inner)
        }
        Component::Paren(inner) | Component::Square(inner) | Component::Curly(inner) => {
            has_env_or_attr(inner)
        }
        _ => false,
    })
}

/// Decide which custom properties can move into the central file without changing any
/// element's cascade: all definitions share one value, sit unconditionally on the root,
/// and the property is not framework-internal. Moving an unreferenced definition is
/// cascade-neutral too (scripts reading the computed value see the same value).
fn hoist_candidates(
    vars: &BTreeMap<String, SiteVar>,
) -> (BTreeMap<String, Vec<Component>>, Vec<KeptVar>) {
    let mut kept: Vec<KeptVar> = Vec::new();
    let mut ok: BTreeMap<String, Vec<Component>> = BTreeMap::new();
    for (name, var) in vars {
        let reason = if INTERNAL_PREFIXES.iter().any(|p| name.starts_with(p)) {
            Some("framework-internal runtime variable")
        } else if var.defs.is_empty() {
            Some("referenced but never defined in the stylesheets (set by script or inline style)")
        } else if var.inline_set {
            Some("also set by an inline style attribute")
        } else if var.registered {
            Some("registered with @property (initial-value/syntax stay with the registration)")
        } else if var.defs.iter().any(|d| d.important) {
            Some("declared !important")
        } else if var.defs.iter().any(|d| !d.root_unconditional) {
            Some("also defined in a scoped or conditional context (theme/media/component override)")
        } else if var
            .defs
            .iter()
            .any(|d| signature(&d.value) != signature(&var.defs[0].value))
        {
            Some("defined with different values")
        } else if var.defs.iter().any(|d| has_env_or_attr(&d.value)) {
            Some("value depends on env()/attr()")
        } else {
            None
        };
        match reason {
            Some(r) => kept.push(KeptVar {
                name: name.clone(),
                reason: r.to_string(),
            }),
            None => {
                ok.insert(name.clone(), css::trim(&var.defs[0].value));
            }
        }
    }
    // A hoisted value may only reference variables that are hoisted too.
    loop {
        let mut dropped = Vec::new();
        for (name, value) in &ok {
            let mut refs = Vec::new();
            references_vars(value, &mut refs);
            if refs.iter().any(|r| !ok.contains_key(r)) {
                dropped.push(name.clone());
            }
        }
        if dropped.is_empty() {
            break;
        }
        for name in dropped {
            ok.remove(&name);
            kept.push(KeptVar {
                name,
                reason: "value references a variable that stays in place".to_string(),
            });
        }
    }
    (ok, kept)
}

// ----- color naming ------------------------------------------------------------

fn srgb_to_oklab(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let lin = |v: u8| {
        let c = f64::from(v) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(r), lin(g), lin(b));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    (
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    )
}

const HUES: &[(f64, &str)] = &[
    (15.0, "red"),
    (45.0, "orange"),
    (75.0, "amber"),
    (110.0, "yellow"),
    (145.0, "lime"),
    (165.0, "green"),
    (195.0, "teal"),
    (225.0, "cyan"),
    (255.0, "sky"),
    (275.0, "blue"),
    (300.0, "indigo"),
    (325.0, "violet"),
    (350.0, "pink"),
    (360.0, "red"),
];

/// (lightness 0..1, chroma, hue degrees, alpha percent) of a color key.
/// (lightness, chroma, hue, alpha percent, sRGB bytes when the color is sRGB).
type Lch = (f64, f64, f64, u32, Option<[u8; 3]>);

fn lch_of(key: &str) -> Option<Lch> {
    if let Some(rest) = key.strip_prefix("srgb:") {
        let parts: Vec<u8> = rest.split(',').filter_map(|p| p.parse().ok()).collect();
        if parts.len() != 4 {
            return None;
        }
        let (l, a, b) = srgb_to_oklab(parts[0], parts[1], parts[2]);
        let chroma = (a * a + b * b).sqrt();
        let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
        let alpha = (f64::from(parts[3]) / 255.0 * 100.0).round() as u32;
        return Some((l, chroma, hue, alpha, Some([parts[0], parts[1], parts[2]])));
    }
    if let Some(rest) = key.strip_prefix("other:oklch(") {
        // Authored forms: `oklch(94.3% 0.019 70)`, `oklch(0.943 0.019 70 / 0.5)`, `... 70deg`.
        let body = rest.trim_end_matches(')');
        let (main, alpha) = match body.split_once('/') {
            Some((m, a)) => (m, Some(a.trim())),
            None => (body, None),
        };
        let parts: Vec<&str> = main.split_whitespace().collect();
        if parts.len() == 3 {
            let num = |s: &str, percent_scale: f64| -> Option<f64> {
                match s.strip_suffix('%') {
                    Some(p) => p.parse::<f64>().ok().map(|v| v / 100.0 * percent_scale),
                    None => s.trim_end_matches("deg").parse().ok(),
                }
            };
            let l = num(parts[0], 1.0)?;
            let c = num(parts[1], 0.4)?;
            let h = num(parts[2], 360.0)?;
            let alpha = match alpha {
                Some(a) => (num(a, 1.0)? * 100.0).round() as u32,
                None => 100,
            };
            return Some((l, c, h.rem_euclid(360.0), alpha, None));
        }
    }
    None
}

fn palette_name(key: &str, precision: u32) -> Option<String> {
    let (l, c, h, alpha, rgb) = lch_of(key)?;
    let base = match rgb {
        Some([255, 255, 255]) if precision == 0 => "white".to_string(),
        Some([0, 0, 0]) if precision == 0 => "black".to_string(),
        // A site variable already owns --color-white/--color-black: say what this one is.
        Some([255, 255, 255]) if precision == 1 => "pure-white".to_string(),
        Some([0, 0, 0]) if precision == 1 => "pure-black".to_string(),
        Some([255, 255, 255] | [0, 0, 0]) => return None,
        _ => {
            let family = if c < 0.025 {
                "neutral"
            } else {
                HUES.iter()
                    .find(|(limit, _)| h < *limit)
                    .map_or("red", |(_, n)| n)
            };
            let shade = match precision {
                0 => ((1.0 - l) * 20.0).round() as i64 * 50,
                1 => ((1.0 - l) * 100.0).round() as i64 * 10,
                _ => ((1.0 - l) * 1000.0).round() as i64,
            };
            let shade = if precision == 0 {
                shade.clamp(50, 950)
            } else {
                shade.clamp(1, 999)
            };
            format!("{family}-{shade}")
        }
    };
    Some(if alpha < 100 {
        format!("{base}-a{alpha}")
    } else {
        base
    })
}

fn role_of(group: &Group) -> Option<&'static str> {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (property, n) in &group.properties {
        let role = match property.as_str() {
            "color"
            | "-webkit-text-fill-color"
            | "caret-color"
            | "text-decoration-color"
            | "text-emphasis-color" => "text",
            p if p.starts_with("background") => "surface",
            p if p.starts_with("border")
                || p.starts_with("outline")
                || p.starts_with("column-rule") =>
            {
                "border"
            }
            "box-shadow" | "text-shadow" => "shadow",
            "fill" | "stroke" | "stop-color" => "graphic",
            _ => "color",
        };
        *counts.entry(role).or_default() += n;
    }
    let total: usize = counts.values().sum();
    let (role, n) = counts.into_iter().max_by_key(|(_, n)| *n)?;
    (role != "color" && n * 10 >= total * 6).then_some(role)
}

// ----- standard names ----------------------------------------------

fn tailwind_text(px: f64) -> Option<&'static str> {
    Some(match px as i64 {
        12 if px == 12.0 => "xs",
        14 if px == 14.0 => "sm",
        16 if px == 16.0 => "base",
        18 if px == 18.0 => "lg",
        20 if px == 20.0 => "xl",
        24 if px == 24.0 => "2xl",
        30 if px == 30.0 => "3xl",
        36 if px == 36.0 => "4xl",
        48 if px == 48.0 => "5xl",
        60 if px == 60.0 => "6xl",
        72 if px == 72.0 => "7xl",
        96 if px == 96.0 => "8xl",
        128 if px == 128.0 => "9xl",
        _ => return None,
    })
}

fn tailwind_radius(x: f64) -> Option<&'static str> {
    const TABLE: &[(f64, &str)] = &[
        (2.0, "xs"),
        (4.0, "sm"),
        (6.0, "md"),
        (8.0, "lg"),
        (12.0, "xl"),
        (16.0, "2xl"),
        (24.0, "3xl"),
        (32.0, "4xl"),
    ];
    TABLE
        .iter()
        .find(|(v, _)| (*v - x).abs() < 1e-9)
        .map(|(_, n)| *n)
}

fn tailwind_weight(n: f64) -> Option<&'static str> {
    Some(match n as i64 {
        100 => "thin",
        200 => "extralight",
        300 => "light",
        400 => "normal",
        500 => "medium",
        600 => "semibold",
        700 => "bold",
        800 => "extrabold",
        900 => "black",
        _ => return None,
    })
}

fn tailwind_leading(x: f64) -> Option<&'static str> {
    const TABLE: &[(f64, &str)] = &[
        (1.0, "none"),
        (1.25, "tight"),
        (1.375, "snug"),
        (1.5, "normal"),
        (1.625, "relaxed"),
        (2.0, "loose"),
    ];
    TABLE
        .iter()
        .find(|(v, _)| (*v - x).abs() < 1e-9)
        .map(|(_, n)| *n)
}

fn tailwind_tracking(em: f64) -> Option<&'static str> {
    Some(match (em * 1000.0).round() as i64 {
        -50 => "tighter",
        -25 => "tight",
        0 => "normal",
        25 => "wide",
        50 => "wider",
        100 => "widest",
        _ => return None,
    })
}

fn tailwind_breakpoint(px: f64) -> Option<&'static str> {
    Some(match px as i64 {
        640 => "sm",
        768 => "md",
        1024 => "lg",
        1280 => "xl",
        1536 => "2xl",
        _ => return None,
    })
}

fn tailwind_ease(key: &str) -> Option<&'static str> {
    Some(match key {
        "cubic-bezier(0.4,0,1,1)" => "in",
        "cubic-bezier(0,0,0.2,1)" => "out",
        "cubic-bezier(0.4,0,0.2,1)" => "in-out",
        _ => return None,
    })
}

fn family_slug(key: &str) -> String {
    let first = key.split(',').next().unwrap_or(key);
    let mut out = String::new();
    for c in first.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn unit_value_px(key: &str) -> Option<f64> {
    key.strip_prefix("px:")?.parse().ok()
}

fn em_value(key: &str) -> Option<f64> {
    key.strip_prefix("em:")?.parse().ok()
}

struct Namer<'a> {
    used: BTreeSet<String>,
    taken_by_site: &'a BTreeSet<String>,
}

impl Namer<'_> {
    fn claim(&mut self, name: &str) -> bool {
        if self.used.contains(name) || self.taken_by_site.contains(name) {
            return false;
        }
        self.used.insert(name.to_string());
        true
    }
}

struct Pending<'a> {
    category: Category,
    key: &'a str,
    group: &'a Group,
}

/// Candidate names for a group, best first. Names are never hashes or sequence numbers.
fn candidates(p: &Pending<'_>, inv: &Inventory) -> Vec<(String, Basis)> {
    let g = p.group;
    let mut out: Vec<(String, Basis)> = Vec::new();
    let subject = g
        .dominant_subject()
        .filter(|(_, share)| *share >= 0.6 && g.decls >= 2)
        .map(|(s, _)| s.to_string());
    let k = (p.category, p.key.to_string());
    match p.category {
        Category::Color => {
            if inv.foreground.as_ref() == Some(&k) {
                out.push(("--color-foreground".into(), Basis::PageRole));
            }
            if inv.background.as_ref() == Some(&k) {
                out.push(("--color-background".into(), Basis::PageRole));
            }
            if let (Some(subject), Some(role)) = (&subject, role_of(g)) {
                out.push((format!("--color-{subject}-{role}"), Basis::UsageSubject));
                if let Some(tone) = palette_name(p.key, 0) {
                    out.push((
                        format!("--color-{subject}-{role}-{tone}"),
                        Basis::UsageSubject,
                    ));
                }
            }
            for precision in 0..=2 {
                if let Some(n) = palette_name(p.key, precision) {
                    out.push((format!("--color-{n}"), Basis::Palette));
                }
            }
        }
        Category::Radius => {
            if let Some(px) = unit_value_px(p.key) {
                if px >= 999.0 {
                    out.push(("--radius-full".into(), Basis::TailwindDefault));
                    out.push((
                        format!(
                            "--radius-full-{}",
                            if px < 1e9 {
                                (px as i64).to_string()
                            } else {
                                "max".into()
                            }
                        ),
                        Basis::Value,
                    ));
                }
                if let Some(subject) = &subject {
                    out.push((format!("--radius-{subject}"), Basis::UsageSubject));
                }
                if let Some(n) = tailwind_radius(px) {
                    out.push((format!("--radius-{n}"), Basis::TailwindDefault));
                }
                if px < 999.0 && px.fract() == 0.0 {
                    out.push((format!("--radius-{}px", px as i64), Basis::Value));
                }
            } else if p.key == "%:50" {
                out.push(("--radius-circle".into(), Basis::Value));
            }
        }
        Category::Shadow => {
            let prefix = if p.key.starts_with("text:") {
                "--text-shadow-"
            } else {
                "--shadow-"
            };
            if let Some(subject) = &subject {
                out.push((format!("{prefix}{subject}"), Basis::UsageSubject));
            }
            if let Some(name) = shadow_value_name(p.key) {
                out.push((format!("{prefix}{name}"), Basis::Value));
            }
        }
        Category::FontSize => {
            if let Some(px) = unit_value_px(p.key)
                && let Some(n) = tailwind_text(px)
            {
                out.push((format!("--text-{n}"), Basis::TailwindDefault));
            }
            if let Some(subject) = &subject
                && subject.len() == 2
                && subject.starts_with('h')
            {
                out.push((format!("--text-{subject}"), Basis::UsageSubject));
            }
            if let Some(px) = unit_value_px(p.key)
                && px.fract() == 0.0
            {
                out.push((format!("--text-{}px", px as i64), Basis::Value));
            }
        }
        Category::FontFamily => {
            let slug = family_slug(p.key);
            if !slug.is_empty() {
                out.push((format!("--font-{slug}"), Basis::Value));
                for generic in ["sans-serif", "serif", "monospace"] {
                    if p.key.split(',').any(|f| f == generic) {
                        out.push((
                            format!(
                                "--font-{slug}-{}",
                                generic.replace("-serif", "").replace("monospace", "mono")
                            ),
                            Basis::Value,
                        ));
                    }
                }
            }
        }
        Category::FontWeight => {
            if let Ok(n) = p.key.parse::<f64>() {
                if let Some(name) = tailwind_weight(n) {
                    out.push((format!("--font-weight-{name}"), Basis::TailwindDefault));
                }
                if n.fract() == 0.0 {
                    out.push((format!("--font-weight-{}", n as i64), Basis::Value));
                }
            }
        }
        Category::LetterSpacing => {
            if let Some(em) = em_value(p.key) {
                let thousandths = (em * 1000.0).round() as i64;
                if let Some(name) = tailwind_tracking(em) {
                    out.push((format!("--tracking-{name}"), Basis::TailwindDefault));
                }
                if let Some(subject) = &subject {
                    out.push((format!("--tracking-{subject}"), Basis::UsageSubject));
                }
                let sign = if thousandths < 0 { "minus-" } else { "" };
                out.push((
                    format!("--tracking-{sign}{}", thousandths.abs()),
                    Basis::Value,
                ));
            } else if let Some(px) = unit_value_px(p.key)
                && px.fract() == 0.0
            {
                let sign = if px < 0.0 { "minus-" } else { "" };
                out.push((
                    format!("--tracking-{sign}{}px", px.abs() as i64),
                    Basis::Value,
                ));
            }
        }
        Category::LineHeight => {
            if let Ok(n) = p.key.parse::<f64>() {
                if let Some(name) = tailwind_leading(n) {
                    out.push((format!("--leading-{name}"), Basis::TailwindDefault));
                }
                if let Some(subject) = &subject {
                    out.push((format!("--leading-{subject}"), Basis::UsageSubject));
                }
                let hundredths = (n * 100.0).round();
                if (hundredths - n * 100.0).abs() < 1e-6 {
                    out.push((format!("--leading-{}", hundredths as i64), Basis::Value));
                } else {
                    out.push((
                        format!("--leading-{}", (n * 1000.0).round() as i64),
                        Basis::Value,
                    ));
                }
            } else if let Some(px) = unit_value_px(p.key)
                && px.fract() == 0.0
            {
                out.push((format!("--leading-{}px", px as i64), Basis::Value));
            }
        }
        Category::Easing => {
            if let Some(n) = tailwind_ease(p.key) {
                out.push((format!("--ease-{n}"), Basis::TailwindDefault));
            }
            if let Some(subject) = &subject {
                out.push((format!("--ease-{subject}"), Basis::UsageSubject));
            }
        }
        Category::Duration => {
            if let Ok(ms) = p.key.parse::<f64>()
                && ms.fract() == 0.0
            {
                out.push((format!("--duration-{}", ms as i64), Basis::Value));
            }
        }
        Category::ZIndex => {
            if let Some(subject) = &subject {
                out.push((format!("--z-{subject}"), Basis::UsageSubject));
            }
        }
        Category::BorderWidth => {
            if let Some(px) = unit_value_px(p.key) {
                if px < 1.0 {
                    out.push(("--border-width-hairline".into(), Basis::Value));
                } else if px.fract() == 0.0 {
                    out.push((format!("--border-width-{}", px as i64), Basis::Value));
                }
            }
        }
        Category::Spacing | Category::Size | Category::Opacity | Category::Breakpoint => {}
    }
    out
}

/// Name a shadow by the geometry of its strongest layer: `y4-blur12`, `inset-y1-blur2`,
/// with `-plus1` per extra layer. Fractional pixels use `_` (`y0_5`). Not a rank, so it never
/// claims to be Tailwind's `sm`/`md`/`lg`.
fn shadow_value_name(key: &str) -> Option<String> {
    let key = key.split_once(':').map_or(key, |(_, rest)| rest);
    let fmt = |v: f64| -> String { fmt_num(v).replace('-', "n").replace('.', "_") };
    let layers: Vec<&str> = key.split(" | ").collect();
    let mut best: Option<(f64, String)> = None;
    for layer in &layers {
        let inset = layer.starts_with("inset ");
        let layer = layer.strip_prefix("inset ").unwrap_or(layer);
        let mut parts = layer.split(' ');
        let _x = unit_value_px(parts.next()?)?;
        let y = unit_value_px(parts.next()?)?;
        let blur = unit_value_px(parts.next()?)?;
        let name = format!(
            "{}y{}-blur{}",
            if inset { "inset-" } else { "" },
            fmt(y),
            fmt(blur)
        );
        let weight = blur + y.abs();
        if best.as_ref().is_none_or(|(w, _)| weight > *w) {
            best = Some((weight, name));
        }
    }
    let (_, mut name) = best?;
    if layers.len() > 1 {
        name.push_str(&format!("-plus{}", layers.len() - 1));
    }
    Some(name)
}

// ----- the plan ----------------------------------------------------------------

fn base_spacing(
    inv: &Inventory,
    site: Option<f64>,
    opts: &Options,
    ctx: &Context,
) -> Option<(f64, f64)> {
    // Returns (base px, coverage) over spacing declarations.
    let groups: Vec<(&Group, f64)> = inv
        .groups
        .iter()
        .filter(|((c, _), _)| *c == Category::Spacing)
        .filter_map(|(_, g)| g.px().map(|px| (g, px)))
        .collect();
    let total: usize = groups.iter().map(|(g, _)| g.decls).sum();
    if total == 0 {
        return site.map(|b| (b, 0.0));
    }
    let _ = (opts, ctx);
    let coverage = |base: f64| {
        let covered: usize = groups
            .iter()
            .filter(|(_, px)| on_scale(*px, base))
            .map(|(g, _)| g.decls)
            .sum();
        covered as f64 / total as f64
    };
    if let Some(b) = site {
        return Some((b, coverage(b)));
    }
    for base in [4.0, 2.0] {
        let c = coverage(base);
        if c >= 0.6 {
            return Some((base, c));
        }
    }
    None
}

fn on_scale(px: f64, base: f64) -> bool {
    let n = px / base;
    let half = (n * 2.0).round() / 2.0;
    (n - half).abs() < 1e-6 && n.abs() <= 96.0 && n.abs() > 0.0
}

impl TokenSet {
    pub fn build(inv: &Inventory, ctx: Context, opts: &Options) -> TokenSet {
        let mut set = TokenSet {
            ctx: Some(ctx),
            ..Default::default()
        };
        let (hoistable, kept) = hoist_candidates(&inv.vars);
        set.kept_vars = kept;

        // Site tokens: hoisted variables with a recognizable design value.
        let mut site_primary: HashMap<(Category, String), (String, usize)> = HashMap::new();
        let mut site_spacing_base: Option<(String, f64)> = None;
        for (name, value) in &hoistable {
            let var = &inv.vars[name];
            let class = classify_var(name, value, &ctx);
            let raw_value = var.defs[0]
                .raw
                .clone()
                .unwrap_or_else(|| css::to_css(value));
            set.hoisted.push(Hoisted {
                name: name.clone(),
                value: raw_value.clone(),
                category: class.as_ref().map(|c| c.0),
                references: var.references,
            });
            if let Some((cat, key, length)) = class {
                if name == "--spacing" {
                    if let Some(px) = length.as_ref().and_then(Length::px) {
                        site_spacing_base = Some((raw_value.clone(), px));
                    }
                    continue;
                }
                let entry = site_primary
                    .entry((cat, key))
                    .or_insert((name.clone(), var.references));
                if var.references > entry.1 || (var.references == entry.1 && *name < entry.0) {
                    *entry = (name.clone(), var.references);
                }
            }
        }
        // Every custom property the site mentions is reserved, hoisted or not: a derived token
        // must never shadow or be shadowed by a variable that stays in place.
        let site_names: BTreeSet<String> = inv.vars.keys().cloned().collect();

        // Spacing scale.
        let site_base_px = site_spacing_base.as_ref().map(|(_, px)| *px);
        let site_owns_spacing = inv.vars.contains_key("--spacing") && site_spacing_base.is_none();
        if site_owns_spacing {
            set.gaps.push("The site defines --spacing in a form that cannot be reused (scoped, themed, not a constant length, or rem with a varying root size), so no spacing scale was derived and spacing lengths stay literal.".into());
        }
        let scale_base = if site_owns_spacing {
            None
        } else {
            base_spacing(inv, site_base_px, opts, &ctx)
        };
        if let Some((base, _coverage)) = scale_base {
            let groups: Vec<&Group> = inv
                .groups
                .iter()
                .filter(|((c, _), _)| *c == Category::Spacing)
                .map(|(_, g)| g)
                .collect();
            let mut members = 0usize;
            let mut off = 0usize;
            let (mut rem, mut px_units) = (0usize, 0usize);
            for g in &groups {
                match g.px() {
                    Some(px) if on_scale(px, base) => {
                        members += g.decls;
                        rem += g.units.get("rem").copied().unwrap_or(0);
                        px_units += g.units.get("px").copied().unwrap_or(0);
                    }
                    _ => off += g.decls,
                }
            }
            let _ = (rem, px_units);
            let (value, site_defined) = match &site_spacing_base {
                Some((v, _)) => (v.clone(), true),
                None => {
                    let use_rem = ctx.rem_px.is_some() && rem > px_units;
                    match (use_rem, ctx.rem_px) {
                        (true, Some(root)) => (format!("{}rem", fmt_num(base / root)), false),
                        _ => (format!("{}px", fmt_num(base)), false),
                    }
                }
            };
            // Declarations are converted between px and rem when the scale unit differs from the
            // unit they were written in (computed-equal at the captured root font size).
            let scale_is_rem = value.ends_with("rem");
            let converted = if scale_is_rem { px_units } else { rem };
            set.spacing = Some(SpacingScale {
                name: "--spacing".into(),
                base_px: base,
                value,
                site_defined,
                members,
                off_scale: off,
                converted_between_px_and_rem: converted,
            });
        }

        // Group the rest.
        let mut pending: Vec<Pending<'_>> = Vec::new();
        for ((category, key), group) in &inv.groups {
            if *category == Category::Spacing
                && !site_primary.contains_key(&(*category, key.clone()))
            {
                continue; // off-token spacing is handled by the scale
            }
            if let Some((name, _)) = site_primary.get(&(*category, key.clone())) {
                set.push_token(Token {
                    name: name.clone(),
                    category: *category,
                    key: key.clone(),
                    value: set
                        .hoisted
                        .iter()
                        .find(|h| &h.name == name)
                        .map(|h| h.value.clone())
                        .unwrap_or_default(),
                    origin: Origin::Site,
                    basis: Basis::SiteName,
                    uses: group.decls,
                    weight: group.weight,
                    properties: group.properties.keys().cloned().collect(),
                    merged_spellings: group.spellings.keys().cloned().collect(),
                    merged_across_units: crosses_units(group, &hoistable[name]),
                });
                continue;
            }
            if group.decls < opts.min_uses {
                *set.single_use.entry(*category).or_default() += 1;
                continue;
            }
            // A literal that mentions a variable can only become a token when that variable is
            // defined in the central file too.
            let dangling = group.spellings.keys().any(|s| {
                let mut refs = Vec::new();
                references_vars(&css::parse_components(s), &mut refs);
                refs.iter().any(|r| !hoistable.contains_key(r))
            });
            if dangling {
                set.unnamed.push(Unnamed {
                    category: *category,
                    value: group.preferred_spelling(),
                    uses: group.decls,
                    reason: "value references a variable that is not defined in the central file"
                        .into(),
                });
                continue;
            }
            pending.push(Pending {
                category: *category,
                key,
                group,
            });
        }

        // Most used first so the best name goes to the most used value.
        pending.sort_by(|a, b| {
            b.group
                .weight
                .cmp(&a.group.weight)
                .then(b.group.decls.cmp(&a.group.decls))
                .then(a.key.cmp(b.key))
        });
        let mut namer = Namer {
            used: set.tokens.iter().map(|t| t.name.clone()).collect(),
            taken_by_site: &site_names,
        };
        for p in &pending {
            let names = candidates(p, inv);
            let chosen = names.into_iter().find(|(n, _)| namer.claim(n));
            match chosen {
                Some((name, basis)) => set.push_token(Token {
                    name,
                    category: p.category,
                    key: p.key.to_string(),
                    value: p.group.preferred_spelling(),
                    origin: Origin::Derived,
                    basis,
                    uses: p.group.decls,
                    weight: p.group.weight,
                    properties: p.group.properties.keys().cloned().collect(),
                    merged_spellings: p.group.spellings.keys().cloned().collect(),
                    merged_across_units: p.group.units.contains_key("px")
                        && p.group.units.contains_key("rem"),
                }),
                None => set.unnamed.push(Unnamed {
                    category: p.category,
                    value: p.group.preferred_spelling(),
                    uses: p.group.decls,
                    reason: no_name_reason(p, inv),
                }),
            }
        }

        // Breakpoints: report and name; media queries stay literal (custom properties are
        // not allowed inside media conditions).
        let mut used: BTreeSet<String> = BTreeSet::new();
        for (px_text, uses) in &inv.breakpoints {
            let Ok(px) = px_text.parse::<f64>() else {
                continue;
            };
            let (name, basis) = match tailwind_breakpoint(px) {
                Some(n) => (format!("--breakpoint-{n}"), Basis::TailwindDefault),
                None => (format!("--breakpoint-{}", px as i64), Basis::Value),
            };
            if used.insert(name.clone()) && !site_names.contains(&name) {
                set.breakpoints.push(Breakpoint {
                    name,
                    px,
                    uses: *uses,
                    basis,
                });
            }
        }
        set.breakpoints.sort_by(|a, b| a.px.total_cmp(&b.px));
        set.tokens.sort_by(|a, b| {
            a.category
                .cmp(&b.category)
                .then_with(|| sort_value(a).total_cmp(&sort_value(b)))
                .then(a.name.cmp(&b.name))
        });
        set.rebuild_index();
        set.hoisted.sort_by(|a, b| a.name.cmp(&b.name));
        set
    }

    fn push_token(&mut self, token: Token) {
        self.tokens.push(token);
    }

    fn rebuild_index(&mut self) {
        self.by_key = self
            .tokens
            .iter()
            .enumerate()
            .map(|(i, t)| ((t.category, t.key.clone()), i))
            .collect();
    }

    pub fn token_for_key(&self, category: Category, key: &str) -> Option<&Token> {
        self.by_key
            .get(&(category, key.to_string()))
            .map(|&i| &self.tokens[i])
    }

    /// Replacement components for a recognized value, or `None` to leave it literal.
    pub fn replacement(&self, hit: &Hit) -> Option<Vec<Component>> {
        if hit.category == Category::Spacing {
            if let Some(token) = self.token_for_key(Category::Spacing, &hit.key) {
                return Some(var_ref(&token.name));
            }
            let scale = self.spacing.as_ref()?;
            let px = hit.length.as_ref()?.px()?;
            if !on_scale(px, scale.base_px) {
                return None;
            }
            let n = px / scale.base_px;
            if (n - 1.0).abs() < 1e-9 {
                return Some(var_ref(&scale.name));
            }
            return Some(css::parse_components(&format!(
                "calc(var({}) * {})",
                scale.name,
                fmt_num(n)
            )));
        }
        self.token_for_key(hit.category, &hit.key)
            .map(|t| var_ref(&t.name))
    }

    /// Look up the token for a CSS property/value pair (used by the Base UI emitters).
    pub fn token_for(&self, property: &str, value: &str) -> Option<&Token> {
        let ctx = self.ctx?;
        let mut found = None;
        super::recognize::visit_value(
            property,
            &css::parse_components(value),
            &ctx,
            &mut |hit: &Hit| {
                if found.is_none() {
                    found = self.token_for_key(hit.category, &hit.key);
                }
                None
            },
        );
        found
    }
}

fn var_ref(name: &str) -> Vec<Component> {
    vec![Component::Func(
        "var".into(),
        vec![Component::Tok(Tok::Ident(name.to_string()))],
    )]
}

fn sort_value(t: &Token) -> f64 {
    unit_value_px(&t.key)
        .or_else(|| em_value(&t.key))
        .or_else(|| t.key.parse().ok())
        .unwrap_or(0.0)
}

fn no_name_reason(p: &Pending<'_>, inv: &Inventory) -> String {
    let g = p.group;
    match p.category {
        Category::Color => match (lch_of(p.key), g.dominant_subject()) {
            (None, _) => "color is not expressible as an sRGB/OKLCH palette name (color(), lab(), color-mix(), hwb())".into(),
            _ => "all palette names for this color are taken by other colors".into(),
        },
        Category::ZIndex => "no component subject dominates (>=60%) the declarations using this z-index; a name would be invented".into(),
        Category::Easing => "non-standard curve and no dominant component subject".into(),
        Category::Shadow => "geometry name unavailable (fractional or relative offsets) and no dominant component subject".into(),
        Category::Radius | Category::FontSize => "fractional or relative size with no standard name; kept literal rather than named by an invented step".into(),
        _ => {
            let _ = inv;
            "no defensible name from usage, standard scale, or value".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::css::parse_sheet;
    use super::*;

    fn plan(text: &str) -> TokenSet {
        let sheet = parse_sheet(text);
        let ctx = Context { rem_px: Some(16.0) };
        let inv = Inventory::collect(&[&sheet], ctx);
        TokenSet::build(&inv, ctx, &Options::default())
    }

    #[test]
    fn site_variables_win_and_literals_map_onto_them() {
        let set = plan(
            ":root { --color-ink: #111 } .a { color: var(--color-ink); border-color: #111 } .b { color: #111 }",
        );
        let t = set.token_for("color", "#111111").expect("token");
        assert_eq!(t.name, "--color-ink");
        assert_eq!(t.origin, Origin::Site);
        assert_eq!(set.hoisted.len(), 1);
    }

    #[test]
    fn scoped_variables_are_never_hoisted_or_used_as_targets() {
        let set = plan(
            ":root { --ink: #111 } .dark { --ink: #eee } .a { color: var(--ink) } .b { color: #111 } .c { color: #111 }",
        );
        assert!(set.hoisted.is_empty());
        assert!(
            set.kept_vars
                .iter()
                .any(|k| k.name == "--ink" && k.reason.contains("scoped"))
        );
        let t = set.token_for("color", "#111").unwrap();
        assert_eq!(
            t.origin,
            Origin::Derived,
            "a themed variable must not absorb literals"
        );
    }

    #[test]
    fn derived_names_use_subjects_then_palette_never_sequences() {
        let set = plan(
            ".nav__a { background-color: #0a0a0a } .nav__b { background-color: #0a0a0a } .x { color: #fff } .y { color: #fff } .z { color: #3b82f6 } .w { color: #3b82f6 }",
        );
        let names: Vec<&str> = set.tokens.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"--color-nav-surface"), "{names:?}");
        assert!(names.contains(&"--color-white"), "{names:?}");
        assert!(
            names.iter().any(|n| n.starts_with("--color-blue-")),
            "{names:?}"
        );
        assert!(
            set.tokens
                .iter()
                .all(|t| !t.name.ends_with("-1") && !t.name.contains("color-2"))
        );
    }

    #[test]
    fn single_use_values_stay_literal_and_are_counted() {
        let set = plan(".a { color: #123456 } .b { color: #abcdef }");
        assert!(set.tokens.is_empty());
        assert_eq!(set.single_use.get(&Category::Color), Some(&2));
    }

    #[test]
    fn spacing_becomes_a_scale_with_off_scale_values_left_alone() {
        let set = plan(
            ".a { padding: 16px 8px } .b { padding: 16px; margin: 12px } .c { gap: 6px; margin-top: 7px }",
        );
        let scale = set.spacing.as_ref().expect("scale");
        assert_eq!(scale.base_px, 4.0);
        assert_eq!(scale.value, "4px");
        assert!(scale.off_scale >= 1, "7px is off a 4px grid");
        let hit = |px: f64| Hit {
            category: Category::Spacing,
            key: format!("px:{}", fmt_num(px)),
            spelling: String::new(),
            property: "padding".into(),
            length: Some(Length::Px(px)),
            unit: Some("px".into()),
        };
        assert_eq!(
            css::to_css(&set.replacement(&hit(16.0)).unwrap()),
            "calc(var(--spacing) * 4)"
        );
        assert_eq!(
            css::to_css(&set.replacement(&hit(4.0)).unwrap()),
            "var(--spacing)"
        );
        assert_eq!(
            css::to_css(&set.replacement(&hit(-8.0)).unwrap()),
            "calc(var(--spacing) * -2)"
        );
        assert!(set.replacement(&hit(7.0)).is_none());
    }

    #[test]
    fn standard_scales_match_by_value_and_everything_else_is_named_by_value() {
        let set = plan(
            ".a { font-size: 14px; border-radius: 4px; font-weight: 500; line-height: 1.5 } .b { font-size: 14px; border-radius: 4px; font-weight: 500; line-height: 1.5 } .c { border-radius: 20px; font-size: 13px } .d { border-radius: 20px; font-size: 13px } .e { font-weight: 360 } .f { font-weight: 360 }",
        );
        let names: BTreeSet<&str> = set.tokens.iter().map(|t| t.name.as_str()).collect();
        for expected in [
            "--text-sm",
            "--radius-sm",
            "--font-weight-medium",
            "--leading-normal",
            "--font-weight-360",
            "--radius-20px",
            "--text-13px",
        ] {
            assert!(names.contains(expected), "{expected} in {names:?}");
        }
        assert!(
            !names.contains("--text-2xl") && !names.contains("--radius-lg"),
            "no invented Tailwind steps: {names:?}"
        );
    }

    #[test]
    fn shadows_are_named_by_geometry_not_by_invented_rank() {
        let set = plan(
            ".a { box-shadow: 0 4px 12px #0003 } .b { box-shadow: 0 4px 12px #0003 } .c { box-shadow: inset 0 1px 0 #fff } .d { box-shadow: inset 0 1px 0 #fff }",
        );
        let names: BTreeSet<&str> = set.tokens.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains("--shadow-y4-blur12"), "{names:?}");
        assert!(names.contains("--shadow-inset-y1-blur0"), "{names:?}");
        assert!(
            !names
                .iter()
                .any(|n| matches!(*n, "--shadow-sm" | "--shadow-md" | "--shadow-lg"))
        );
    }

    #[test]
    fn derived_names_never_collide_with_variables_that_stay_in_place() {
        // --color-white is themed (kept in place); the derived white token must not reuse the name.
        let set = plan(
            ":root { --color-white: #fff } .dark { --color-white: #000 } .a { color: #fff } .b { color: #fff }",
        );
        let t = set.token_for("color", "#fff").unwrap();
        assert_ne!(t.name, "--color-white");
        // A site-owned --spacing that cannot be reused disables the derived scale.
        let set =
            plan(".card { --spacing: 10px } .a { padding-top: 16px } .b { padding-top: 16px }");
        assert!(set.spacing.is_none());
        assert!(set.gaps.iter().any(|g| g.contains("--spacing")));
    }

    #[test]
    fn unnamed_values_carry_a_reason() {
        let set = plan(".a { z-index: 40 } .b { z-index: 40 }");
        assert_eq!(set.unnamed.len(), 1);
        assert!(set.unnamed[0].reason.contains("subject"));
        let set = plan(".nav { z-index: 40 } .nav__panel { z-index: 40 }");
        assert!(set.tokens.iter().any(|t| t.name == "--z-nav"));
    }
}
