//! Evaluation of `@media` and `@supports` conditions against a captured environment.
//!
//! Results are three-valued ([`Tri`]); anything this module does not understand is
//! `Unknown`, never silently true or false. The assumptions that are baked in
//! (`hover: hover`, `pointer: fine`, no reduced motion, ...) are listed in
//! [`Environment::assumptions`] and copied into the token report.

use super::css::{Component, Tok};
use super::selector::Tri;
use super::value::split_commas;

#[derive(Debug, Clone)]
pub struct Environment {
    pub width: f64,
    pub height: f64,
    pub color_scheme: String,
    /// `prefers-reduced-motion: reduce` (the default is `no-preference`, the capture's setting).
    pub reduced_motion: bool,
}

impl Environment {
    pub fn assumptions() -> Vec<&'static str> {
        vec![
            "media: hover/any-hover = hover, pointer/any-pointer = fine (desktop headless capture)",
            "media: prefers-reduced-motion = no-preference, prefers-contrast = no-preference, forced-colors = none",
            "media: resolution and device-pixel-ratio are Unknown (not recorded in the capture)",
            "media: em/rem in queries use the 16px initial font size",
            "supports: unprefixed property declarations are assumed supported unless listed as unsupported; vendor-prefixed and selector()/font-tech() tests are Unknown",
            "container, scope and starting-style conditions are Unknown",
            "pseudo-classes such as :hover/:focus/:active are false (resting snapshot)",
        ]
    }
}

fn not(t: Tri) -> Tri {
    match t {
        Tri::Yes => Tri::No,
        Tri::No => Tri::Yes,
        Tri::Unknown => Tri::Unknown,
    }
}
fn and(a: Tri, b: Tri) -> Tri {
    match (a, b) {
        (Tri::No, _) | (_, Tri::No) => Tri::No,
        (Tri::Yes, Tri::Yes) => Tri::Yes,
        _ => Tri::Unknown,
    }
}
fn or(a: Tri, b: Tri) -> Tri {
    match (a, b) {
        (Tri::Yes, _) | (_, Tri::Yes) => Tri::Yes,
        (Tri::No, Tri::No) => Tri::No,
        _ => Tri::Unknown,
    }
}

fn items(components: &[Component]) -> Vec<&Component> {
    components.iter().filter(|c| !c.is_whitespace()).collect()
}

/// Evaluate a media query list (comma separated = OR).
pub fn eval_media(prelude: &[Component], env: &Environment) -> Tri {
    if prelude.iter().all(Component::is_whitespace) {
        return Tri::Yes;
    }
    let mut result = Tri::No;
    for query in split_commas(prelude) {
        result = or(result, eval_query(&query, env));
    }
    result
}

fn eval_query(query: &[Component], env: &Environment) -> Tri {
    let parts = items(query);
    let mut index = 0;
    let mut negate = false;
    if parts
        .get(index)
        .and_then(|c| c.ident())
        .is_some_and(|i| i.eq_ignore_ascii_case("not"))
    {
        negate = true;
        index += 1;
    } else if parts
        .get(index)
        .and_then(|c| c.ident())
        .is_some_and(|i| i.eq_ignore_ascii_case("only"))
    {
        index += 1;
    }
    let mut result = Tri::Yes;
    if let Some(name) = parts.get(index).and_then(|c| c.ident()) {
        let lower = name.to_ascii_lowercase();
        if lower != "not" {
            result = match lower.as_str() {
                "all" | "screen" => Tri::Yes,
                "print" | "speech" | "tty" | "tv" | "projection" | "handheld" | "braille"
                | "embossed" | "aural" => Tri::No,
                _ => Tri::Unknown,
            };
            index += 1;
            if parts
                .get(index)
                .and_then(|c| c.ident())
                .is_some_and(|i| i.eq_ignore_ascii_case("and"))
            {
                index += 1;
            }
        }
    }
    if index < parts.len() {
        result = and(result, eval_condition(&parts[index..], env));
    }
    if negate { not(result) } else { result }
}

/// `term (and term)*`, `term (or term)*`, `not term`.
fn eval_condition(parts: &[&Component], env: &Environment) -> Tri {
    let mut result: Option<Tri> = None;
    let mut op: Option<bool> = None; // Some(true) = and, Some(false) = or
    let mut index = 0;
    while index < parts.len() {
        let mut negate = false;
        if parts[index]
            .ident()
            .is_some_and(|i| i.eq_ignore_ascii_case("not"))
        {
            negate = true;
            index += 1;
        }
        let Some(term) = parts.get(index) else {
            return Tri::Unknown;
        };
        let value = match term {
            Component::Paren(inner) => eval_in_parens(inner, env),
            Component::Func(_, _) => Tri::Unknown, // style(), selector(), ...
            _ => Tri::Unknown,
        };
        let value = if negate { not(value) } else { value };
        result = Some(match (result, op) {
            (None, _) => value,
            (Some(r), Some(true)) => and(r, value),
            (Some(r), Some(false)) => or(r, value),
            (Some(_), None) => return Tri::Unknown,
        });
        index += 1;
        match parts.get(index) {
            None => break,
            Some(c) => match c.ident().map(str::to_ascii_lowercase).as_deref() {
                Some("and") => op = Some(true),
                Some("or") => op = Some(false),
                _ => return Tri::Unknown,
            },
        }
        index += 1;
    }
    result.unwrap_or(Tri::Unknown)
}

fn eval_in_parens(inner: &[Component], env: &Environment) -> Tri {
    let parts = items(inner);
    match parts.first() {
        Some(Component::Paren(_)) => return eval_condition(&parts, env),
        Some(c) if c.ident().is_some_and(|i| i.eq_ignore_ascii_case("not")) => {
            return eval_condition(&parts, env);
        }
        _ => {}
    }
    // Plain feature: `(name)`, `(name: value)` or a range expression.
    if let [Component::Tok(Tok::Ident(name))] = parts.as_slice() {
        return boolean_feature(&name.to_ascii_lowercase(), env);
    }
    if let [
        Component::Tok(Tok::Ident(name)),
        Component::Tok(Tok::Colon),
        value @ ..,
    ] = parts.as_slice()
    {
        return feature(&name.to_ascii_lowercase(), value, env);
    }
    range(&parts, env)
}

fn media_len(c: &Component) -> Option<f64> {
    match c {
        Component::Tok(Tok::Dimension { value, unit, .. }) => {
            let v = f64::from(*value);
            match unit.to_ascii_lowercase().as_str() {
                "px" => Some(v),
                "em" | "rem" => Some(v * 16.0),
                "pt" => Some(v * 96.0 / 72.0),
                "in" => Some(v * 96.0),
                "cm" => Some(v * 96.0 / 2.54),
                "mm" => Some(v * 96.0 / 25.4),
                _ => None,
            }
        }
        Component::Tok(Tok::Number { value, .. }) if *value == 0.0 => Some(0.0),
        _ => None,
    }
}

fn boolean_feature(name: &str, env: &Environment) -> Tri {
    match name {
        "hover" | "any-hover" | "color" | "width" | "height" | "color-gamut" | "pointer"
        | "any-pointer" => Tri::Yes,
        _ => {
            let _ = env;
            Tri::Unknown
        }
    }
}

fn feature(name: &str, value: &[&Component], env: &Environment) -> Tri {
    let ident = match value {
        [Component::Tok(Tok::Ident(i))] => Some(i.to_ascii_lowercase()),
        _ => None,
    };
    let compare = |actual: f64, min: bool, max: bool| -> Tri {
        let Some(v) = value.first().and_then(|c| media_len(c)) else {
            return Tri::Unknown;
        };
        if value.len() != 1 {
            return Tri::Unknown;
        }
        if min {
            (actual >= v).into()
        } else if max {
            (actual <= v).into()
        } else {
            ((actual - v).abs() < 1e-9).into()
        }
    };
    match name {
        "width" | "min-width" | "max-width" => compare(
            env.width,
            name.starts_with("min-"),
            name.starts_with("max-"),
        ),
        "height" | "min-height" | "max-height" => compare(
            env.height,
            name.starts_with("min-"),
            name.starts_with("max-"),
        ),
        "orientation" => match ident.as_deref() {
            Some("portrait") => (env.height >= env.width).into(),
            Some("landscape") => (env.width > env.height).into(),
            _ => Tri::Unknown,
        },
        "prefers-color-scheme" => match ident.as_deref() {
            Some(scheme) => (env.color_scheme.eq_ignore_ascii_case(scheme)).into(),
            None => Tri::Unknown,
        },
        "prefers-reduced-motion" => ident.map_or(Tri::Unknown, |i| match i.as_str() {
            "reduce" => env.reduced_motion.into(),
            "no-preference" => (!env.reduced_motion).into(),
            _ => Tri::Unknown,
        }),
        "prefers-contrast" => ident.map_or(Tri::Unknown, |i| (i == "no-preference").into()),
        "prefers-reduced-transparency" | "prefers-reduced-data" => {
            ident.map_or(Tri::Unknown, |i| (i == "no-preference").into())
        }
        "forced-colors" | "inverted-colors" => ident.map_or(Tri::Unknown, |i| (i == "none").into()),
        "hover" | "any-hover" => ident.map_or(Tri::Unknown, |i| (i == "hover").into()),
        "pointer" | "any-pointer" => ident.map_or(Tri::Unknown, |i| (i == "fine").into()),
        "display-mode" => ident.map_or(Tri::Unknown, |i| (i == "browser").into()),
        "scripting" => ident.map_or(Tri::Unknown, |i| (i == "enabled").into()),
        "update" => ident.map_or(Tri::Unknown, |i| (i == "fast").into()),
        _ => Tri::Unknown,
    }
}

/// Range syntax: `width >= 40rem`, `40rem <= width < 60rem`.
fn range(parts: &[&Component], env: &Environment) -> Tri {
    // Split into operands and operators. Operators are runs of `<`, `>`, `=` delims.
    let mut operands: Vec<&Component> = Vec::new();
    let mut ops: Vec<String> = Vec::new();
    let mut pending = String::new();
    for c in parts {
        match c {
            Component::Tok(Tok::Delim(d @ ('<' | '>' | '='))) => pending.push(*d),
            other => {
                if !pending.is_empty() {
                    ops.push(std::mem::take(&mut pending));
                }
                operands.push(other);
            }
        }
    }
    if !pending.is_empty() || operands.len() != ops.len() + 1 || ops.is_empty() || ops.len() > 2 {
        return Tri::Unknown;
    }
    let resolve = |c: &Component| -> Option<f64> {
        match c.ident().map(str::to_ascii_lowercase).as_deref() {
            Some("width") => Some(env.width),
            Some("height") => Some(env.height),
            Some(_) => None,
            None => media_len(c),
        }
    };
    let mut result = Tri::Yes;
    for (i, op) in ops.iter().enumerate() {
        let (Some(left), Some(right)) = (resolve(operands[i]), resolve(operands[i + 1])) else {
            return Tri::Unknown;
        };
        let holds = match op.as_str() {
            "<" => left < right,
            "<=" => left <= right,
            ">" => left > right,
            ">=" => left >= right,
            "=" => (left - right).abs() < 1e-9,
            _ => return Tri::Unknown,
        };
        result = and(result, holds.into());
    }
    result
}

// ----- @supports ----------------------------------------------------------

/// Properties that Chromium does not support and sites commonly test for.
const UNSUPPORTED_PROPERTIES: &[&str] = &["margin-trim", "masonry-auto-flow"];

pub fn eval_supports(prelude: &[Component]) -> Tri {
    let parts = items(prelude);
    supports_condition(&parts)
}

fn supports_condition(parts: &[&Component]) -> Tri {
    let mut result: Option<Tri> = None;
    let mut op: Option<bool> = None;
    let mut index = 0;
    while index < parts.len() {
        let mut negate = false;
        if parts[index]
            .ident()
            .is_some_and(|i| i.eq_ignore_ascii_case("not"))
        {
            negate = true;
            index += 1;
        }
        let Some(term) = parts.get(index) else {
            return Tri::Unknown;
        };
        let value = match term {
            Component::Paren(inner) => supports_in_parens(inner),
            _ => Tri::Unknown, // selector(), font-tech(), at-rule()
        };
        let value = if negate { not(value) } else { value };
        result = Some(match (result, op) {
            (None, _) => value,
            (Some(r), Some(true)) => and(r, value),
            (Some(r), Some(false)) => or(r, value),
            (Some(_), None) => return Tri::Unknown,
        });
        index += 1;
        match parts.get(index) {
            None => break,
            Some(c) => match c.ident().map(str::to_ascii_lowercase).as_deref() {
                Some("and") => op = Some(true),
                Some("or") => op = Some(false),
                _ => return Tri::Unknown,
            },
        }
        index += 1;
    }
    result.unwrap_or(Tri::Unknown)
}

fn supports_in_parens(inner: &[Component]) -> Tri {
    let parts = items(inner);
    match parts.first() {
        Some(Component::Paren(_)) => return supports_condition(&parts),
        Some(c) if c.ident().is_some_and(|i| i.eq_ignore_ascii_case("not")) => {
            return supports_condition(&parts);
        }
        _ => {}
    }
    let [
        Component::Tok(Tok::Ident(name)),
        Component::Tok(Tok::Colon),
        value @ ..,
    ] = parts.as_slice()
    else {
        return Tri::Unknown;
    };
    let name = name.to_ascii_lowercase();
    if name.starts_with("-moz-") || name.starts_with("-ms-") || name.starts_with("-o-") {
        return Tri::No; // other engines' prefixes: not supported by Chromium
    }
    if name.starts_with('-') && !name.starts_with("--") {
        return Tri::Unknown; // -webkit-*: depends on the property
    }
    if UNSUPPORTED_PROPERTIES.contains(&name.as_str()) {
        return Tri::No;
    }
    if name.starts_with("--") {
        return Tri::Yes;
    }
    // Value functions are only claimed supported for the modern color syntaxes sites test.
    for c in value {
        if let Component::Func(f, _) = c {
            let f = f.to_ascii_lowercase();
            if !matches!(
                f.as_str(),
                "color-mix"
                    | "oklch"
                    | "oklab"
                    | "lab"
                    | "lch"
                    | "rgb"
                    | "hsl"
                    | "calc"
                    | "var"
                    | "min"
                    | "max"
                    | "clamp"
            ) {
                return Tri::Unknown;
            }
        }
    }
    Tri::Yes
}

#[cfg(test)]
mod tests {
    use super::super::css::parse_components;
    use super::*;

    fn env(width: f64) -> Environment {
        Environment {
            width,
            height: 900.0,
            color_scheme: "dark".into(),
            reduced_motion: false,
        }
    }
    fn media(q: &str, width: f64) -> Tri {
        eval_media(&parse_components(q), &env(width))
    }

    #[test]
    fn width_queries_in_both_syntaxes() {
        assert_eq!(media("(min-width: 768px)", 1440.0), Tri::Yes);
        assert_eq!(media("(min-width: 768px)", 375.0), Tri::No);
        assert_eq!(media("(width >= 40rem)", 640.0), Tri::Yes);
        assert_eq!(media("(width >= 40rem)", 639.0), Tri::No);
        assert_eq!(media("(400px <= width < 700px)", 700.0), Tri::No);
        assert_eq!(media("(400px <= width < 700px)", 699.0), Tri::Yes);
        assert_eq!(
            media("screen and (max-width: 600px), print", 375.0),
            Tri::Yes
        );
        assert_eq!(media("not all and (min-width: 1000px)", 1440.0), Tri::No);
    }

    #[test]
    fn preferences_and_unknowns() {
        assert_eq!(media("(prefers-color-scheme: dark)", 1440.0), Tri::Yes);
        assert_eq!(media("(prefers-color-scheme: light)", 1440.0), Tri::No);
        assert_eq!(media("(prefers-reduced-motion: reduce)", 1440.0), Tri::No);
        assert_eq!(
            media("(prefers-reduced-motion: no-preference)", 1440.0),
            Tri::Yes
        );
        assert_eq!(
            media("(hover: hover) and (pointer: fine)", 1440.0),
            Tri::Yes
        );
        assert_eq!(media("(min-resolution: 2dppx)", 1440.0), Tri::Unknown);
        assert_eq!(
            media("(width >= 40rem) and (min-resolution: 2dppx)", 100.0),
            Tri::No
        );
        assert_eq!(media("print", 1440.0), Tri::No);
    }

    #[test]
    fn supports_is_explicit_about_what_it_cannot_know() {
        let s = |q: &str| eval_supports(&parse_components(q));
        assert_eq!(s("(display: grid)"), Tri::Yes);
        assert_eq!(s("(margin-trim: inline)"), Tri::No);
        assert_eq!(s("(-webkit-hyphens: none)"), Tri::Unknown);
        assert_eq!(s("(color: color-mix(in lab, red, red))"), Tri::Yes);
        assert_eq!(
            s(
                "(((-webkit-hyphens:none)) and (not (margin-trim:inline))) or ((-moz-orient:inline) and (not (color:rgb(from red r g b))))"
            ),
            Tri::Unknown
        );
        assert_eq!(s("selector(:has(a))"), Tri::Unknown);
    }
}
