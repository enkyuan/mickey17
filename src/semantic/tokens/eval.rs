//! Value evaluation used to compare cascade results with captured computed styles:
//! shorthand expansion for the box-model shorthands, `var()` substitution, `calc()`
//! over lengths, and canonical forms per property.
//!
//! Everything returns `None` ("not comparable") instead of guessing.

use super::css::{self, Component, Tok};
use super::value::{self, Color, Context, Length, fmt_num};

// ----- shorthand expansion --------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Expansion {
    /// Fully expanded into longhands.
    Longhands(Vec<(String, Vec<Component>)>),
    /// A shorthand whose longhands we do not expand: these longhands get an opaque value.
    Opaque(Vec<&'static str>),
    /// Not a shorthand.
    Not,
}

fn parts(value: &[Component]) -> Vec<Vec<Component>> {
    // Split a space-separated value into top-level component groups.
    value
        .iter()
        .filter(|c| !c.is_whitespace())
        .map(|c| vec![c.clone()])
        .collect()
}

fn four(values: &[Vec<Component>]) -> Option<[Vec<Component>; 4]> {
    match values.len() {
        1 => Some([
            values[0].clone(),
            values[0].clone(),
            values[0].clone(),
            values[0].clone(),
        ]),
        2 => Some([
            values[0].clone(),
            values[1].clone(),
            values[0].clone(),
            values[1].clone(),
        ]),
        3 => Some([
            values[0].clone(),
            values[1].clone(),
            values[2].clone(),
            values[1].clone(),
        ]),
        4 => Some([
            values[0].clone(),
            values[1].clone(),
            values[2].clone(),
            values[3].clone(),
        ]),
        _ => None,
    }
}

fn box_sides(prefix: &str, suffix: &str, value: &[Component]) -> Option<Expansion> {
    let v = four(&parts(value))?;
    let sides = ["top", "right", "bottom", "left"];
    Some(Expansion::Longhands(
        sides
            .iter()
            .zip(v)
            .map(|(side, val)| {
                let name = if prefix == "inset" {
                    (*side).to_string()
                } else {
                    format!("{prefix}-{side}{suffix}")
                };
                (name, val)
            })
            .collect(),
    ))
}

fn pair(a: &str, b: &str, value: &[Component]) -> Option<Expansion> {
    let v = parts(value);
    let (x, y) = match v.len() {
        1 => (v[0].clone(), v[0].clone()),
        2 => (v[0].clone(), v[1].clone()),
        _ => return None,
    };
    Some(Expansion::Longhands(vec![
        (a.to_string(), x),
        (b.to_string(), y),
    ]))
}

fn border_side(side: &str, value: &[Component]) -> Option<Expansion> {
    // `border-top: 1px solid #fff` in any order; missing parts reset to initial (opaque).
    let mut width = None;
    let mut style = None;
    let mut color = None;
    for c in value.iter().filter(|c| !c.is_whitespace()) {
        if parse_border_style(c) {
            style = Some(vec![c.clone()]);
        } else if value::parse_color(c).is_some()
            || c.ident()
                .is_some_and(|i| i.eq_ignore_ascii_case("currentcolor"))
        {
            color = Some(vec![c.clone()]);
        } else if matches!(
            c,
            Component::Tok(Tok::Dimension { .. } | Tok::Number { .. })
        ) || c
            .ident()
            .is_some_and(|i| matches!(i.to_ascii_lowercase().as_str(), "thin" | "medium" | "thick"))
            || matches!(c, Component::Func(..))
        {
            width = Some(vec![c.clone()]);
        } else {
            return None;
        }
    }
    let initial = |v: &str| css::parse_components(v);
    Some(Expansion::Longhands(vec![
        (
            format!("border-{side}-width"),
            width.unwrap_or_else(|| initial("medium")),
        ),
        (
            format!("border-{side}-style"),
            style.unwrap_or_else(|| initial("none")),
        ),
        (
            format!("border-{side}-color"),
            color.unwrap_or_else(|| initial("currentcolor")),
        ),
    ]))
}

fn parse_border_style(c: &Component) -> bool {
    c.ident().is_some_and(|i| {
        matches!(
            i.to_ascii_lowercase().as_str(),
            "none"
                | "hidden"
                | "dotted"
                | "dashed"
                | "solid"
                | "double"
                | "groove"
                | "ridge"
                | "inset"
                | "outset"
        )
    })
}

/// Does the value contain `var()`, `env()` or `attr()` at the top level? Such values
/// cannot be split into components before substitution.
fn has_unresolved(value: &[Component]) -> bool {
    fn walk(c: &Component) -> bool {
        match c {
            Component::Func(name, inner) => {
                matches!(name.to_ascii_lowercase().as_str(), "var" | "env" | "attr")
                    || inner.iter().any(walk)
            }
            Component::Paren(inner) | Component::Square(inner) | Component::Curly(inner) => {
                inner.iter().any(walk)
            }
            _ => false,
        }
    }
    value.iter().any(walk)
}

const OPAQUE_BACKGROUND: &[&str] = &[
    "background-color",
    "background-image",
    "background-position-x",
    "background-position-y",
    "background-size",
    "background-repeat",
    "background-origin",
    "background-clip",
    "background-attachment",
];
const OPAQUE_FONT: &[&str] = &[
    "font-style",
    "font-variant-caps",
    "font-weight",
    "font-stretch",
    "font-size",
    "line-height",
    "font-family",
];
const OPAQUE_TRANSITION: &[&str] = &[
    "transition-property",
    "transition-duration",
    "transition-timing-function",
    "transition-delay",
    "transition-behavior",
];
const OPAQUE_ANIMATION: &[&str] = &[
    "animation-name",
    "animation-duration",
    "animation-timing-function",
    "animation-delay",
    "animation-iteration-count",
    "animation-direction",
    "animation-fill-mode",
    "animation-play-state",
];
const OPAQUE_TEXT_DECORATION: &[&str] = &[
    "text-decoration-line",
    "text-decoration-style",
    "text-decoration-color",
    "text-decoration-thickness",
];
const OPAQUE_OUTLINE: &[&str] = &["outline-color", "outline-style", "outline-width"];
const OPAQUE_FLEX: &[&str] = &["flex-grow", "flex-shrink", "flex-basis"];
const OPAQUE_FLEX_FLOW: &[&str] = &["flex-direction", "flex-wrap"];
const OPAQUE_BORDER_ALL: &[&str] = &[
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "border-top-style",
    "border-right-style",
    "border-bottom-style",
    "border-left-style",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "border-image-source",
];
const OPAQUE_LIST_STYLE: &[&str] = &["list-style-type", "list-style-position", "list-style-image"];
const OPAQUE_OVERFLOW: &[&str] = &["overflow-x", "overflow-y"];
const OPAQUE_PLACE: &[&str] = &["align-content", "justify-content"];
const OPAQUE_MASK: &[&str] = &[
    "mask-image",
    "mask-position",
    "mask-size",
    "mask-repeat",
    "mask-origin",
    "mask-clip",
    "mask-composite",
    "mask-mode",
];
const OPAQUE_COLUMNS: &[&str] = &["column-width", "column-count"];
const OPAQUE_GRID_AREA: &[&str] = &[
    "grid-row-start",
    "grid-column-start",
    "grid-row-end",
    "grid-column-end",
];
const OPAQUE_GRID_TEMPLATE: &[&str] = &[
    "grid-template-rows",
    "grid-template-columns",
    "grid-template-areas",
];
const OPAQUE_INSET_LOGICAL: &[&str] = &[];

/// Expand one declaration for validation. A value that still contains `var()` is
/// substituted first by the caller; unresolved values are not expanded.
pub fn expand(name: &str, value: &[Component]) -> Expansion {
    let value_ok = !has_unresolved(value);
    let simple = |e: Option<Expansion>| -> Expansion {
        match e {
            Some(e) if value_ok => e,
            _ => Expansion::Opaque(opaque_of(name)),
        }
    };
    match name {
        "margin" => simple(box_sides("margin", "", value)),
        "padding" => simple(box_sides("padding", "", value)),
        "inset" => simple(box_sides("inset", "", value)),
        "scroll-margin" => simple(box_sides("scroll-margin", "", value)),
        "scroll-padding" => simple(box_sides("scroll-padding", "", value)),
        "border-width" => simple(box_sides("border", "-width", value)),
        "border-style" => simple(box_sides("border", "-style", value)),
        "border-color" => simple(box_sides("border", "-color", value)),
        "gap" | "grid-gap" => simple(pair("row-gap", "column-gap", value)),
        "margin-inline" => simple(pair("margin-left", "margin-right", value)),
        "margin-block" => simple(pair("margin-top", "margin-bottom", value)),
        "padding-inline" => simple(pair("padding-left", "padding-right", value)),
        "padding-block" => simple(pair("padding-top", "padding-bottom", value)),
        "inset-inline" => simple(pair("left", "right", value)),
        "inset-block" => simple(pair("top", "bottom", value)),
        "margin-inline-start" => Expansion::Longhands(vec![("margin-left".into(), value.to_vec())]),
        "margin-inline-end" => Expansion::Longhands(vec![("margin-right".into(), value.to_vec())]),
        "margin-block-start" => Expansion::Longhands(vec![("margin-top".into(), value.to_vec())]),
        "margin-block-end" => Expansion::Longhands(vec![("margin-bottom".into(), value.to_vec())]),
        "padding-inline-start" => {
            Expansion::Longhands(vec![("padding-left".into(), value.to_vec())])
        }
        "padding-inline-end" => {
            Expansion::Longhands(vec![("padding-right".into(), value.to_vec())])
        }
        "padding-block-start" => Expansion::Longhands(vec![("padding-top".into(), value.to_vec())]),
        "padding-block-end" => {
            Expansion::Longhands(vec![("padding-bottom".into(), value.to_vec())])
        }
        "inset-inline-start" => Expansion::Longhands(vec![("left".into(), value.to_vec())]),
        "inset-inline-end" => Expansion::Longhands(vec![("right".into(), value.to_vec())]),
        "inset-block-start" => Expansion::Longhands(vec![("top".into(), value.to_vec())]),
        "inset-block-end" => Expansion::Longhands(vec![("bottom".into(), value.to_vec())]),
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            simple(border_side(&name["border-".len()..], value))
        }
        "border" => match (value_ok, border_side("top", value)) {
            (true, Some(Expansion::Longhands(top))) => {
                let mut out = Vec::new();
                for side in ["top", "right", "bottom", "left"] {
                    for (n, v) in &top {
                        out.push((n.replacen("top", side, 1), v.clone()));
                    }
                }
                // `border` also resets border-image; keep it opaque rather than invent a value.
                Expansion::Longhands(out)
            }
            _ => Expansion::Opaque(opaque_of(name)),
        },
        "border-radius" => {
            // Only the single-radius form (no `/`) is expanded.
            let slash = value
                .iter()
                .any(|c| matches!(c, Component::Tok(Tok::Delim('/'))));
            if slash || !value_ok {
                return Expansion::Opaque(opaque_of(name));
            }
            match four(&parts(value)) {
                Some([a, b, c, d]) => Expansion::Longhands(vec![
                    ("border-top-left-radius".into(), a),
                    ("border-top-right-radius".into(), b),
                    ("border-bottom-right-radius".into(), c),
                    ("border-bottom-left-radius".into(), d),
                ]),
                None => Expansion::Opaque(opaque_of(name)),
            }
        }
        "background"
        | "font"
        | "transition"
        | "animation"
        | "text-decoration"
        | "outline"
        | "flex"
        | "flex-flow"
        | "list-style"
        | "overflow"
        | "place-content"
        | "mask"
        | "-webkit-mask"
        | "columns"
        | "grid-area"
        | "grid-template"
        | "grid"
        | "place-items"
        | "place-self"
        | "border-image"
        | "text-emphasis"
        | "container"
        | "contain-intrinsic-size"
        | "font-variant"
        | "scroll-snap-type"
        | "overscroll-behavior"
        | "mask-border" => Expansion::Opaque(opaque_of(name)),
        _ => Expansion::Not,
    }
}

pub fn opaque_of(name: &str) -> Vec<&'static str> {
    let list: &[&str] = match name {
        "background" => OPAQUE_BACKGROUND,
        "font" => OPAQUE_FONT,
        "transition" => OPAQUE_TRANSITION,
        "animation" => OPAQUE_ANIMATION,
        "text-decoration" => OPAQUE_TEXT_DECORATION,
        "outline" => OPAQUE_OUTLINE,
        "flex" => OPAQUE_FLEX,
        "flex-flow" => OPAQUE_FLEX_FLOW,
        "border" => OPAQUE_BORDER_ALL,
        "list-style" => OPAQUE_LIST_STYLE,
        "overflow" => OPAQUE_OVERFLOW,
        "place-content" => OPAQUE_PLACE,
        "mask" | "-webkit-mask" => OPAQUE_MASK,
        "columns" => OPAQUE_COLUMNS,
        "grid-area" => OPAQUE_GRID_AREA,
        "grid-template" | "grid" => OPAQUE_GRID_TEMPLATE,
        "inset" | "inset-inline" | "inset-block" => OPAQUE_INSET_LOGICAL,
        "margin" => &["margin-top", "margin-right", "margin-bottom", "margin-left"],
        "padding" => &[
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
        ],
        "border-width" => &[
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
        ],
        "border-style" => &[
            "border-top-style",
            "border-right-style",
            "border-bottom-style",
            "border-left-style",
        ],
        "border-color" => &[
            "border-top-color",
            "border-right-color",
            "border-bottom-color",
            "border-left-color",
        ],
        "border-top" => &["border-top-width", "border-top-style", "border-top-color"],
        "border-right" => &[
            "border-right-width",
            "border-right-style",
            "border-right-color",
        ],
        "border-bottom" => &[
            "border-bottom-width",
            "border-bottom-style",
            "border-bottom-color",
        ],
        "border-left" => &[
            "border-left-width",
            "border-left-style",
            "border-left-color",
        ],
        "border-radius" => &[
            "border-top-left-radius",
            "border-top-right-radius",
            "border-bottom-right-radius",
            "border-bottom-left-radius",
        ],
        "gap" | "grid-gap" => &["row-gap", "column-gap"],
        "margin-inline" => &["margin-left", "margin-right"],
        "margin-block" => &["margin-top", "margin-bottom"],
        "padding-inline" => &["padding-left", "padding-right"],
        "padding-block" => &["padding-top", "padding-bottom"],
        _ => &[],
    };
    list.to_vec()
}

// ----- var() and calc() -------------------------------------------------------

pub fn needs_substitution(value: &[Component]) -> bool {
    has_unresolved(value)
}

/// Replace every `var(--x, fallback)` using `lookup`. `None` if a variable is missing
/// without fallback or the substitution does not terminate.
const MAX_SUBSTITUTED_COMPONENTS: usize = 20_000;

pub fn substitute_vars(
    value: &[Component],
    lookup: &dyn Fn(&str) -> Option<Vec<Component>>,
    depth: usize,
) -> Option<Vec<Component>> {
    let mut budget = MAX_SUBSTITUTED_COMPONENTS;
    substitute_budget(value, lookup, depth, &mut budget)
}

fn count(value: &[Component]) -> usize {
    value
        .iter()
        .map(|c| match c {
            Component::Func(_, i)
            | Component::Paren(i)
            | Component::Square(i)
            | Component::Curly(i) => 1 + count(i),
            _ => 1,
        })
        .sum()
}

/// `budget` bounds the total size produced, so exponentially self-referencing variables
/// fail fast instead of hanging.
fn substitute_budget(
    value: &[Component],
    lookup: &dyn Fn(&str) -> Option<Vec<Component>>,
    depth: usize,
    budget: &mut usize,
) -> Option<Vec<Component>> {
    if depth > 16 {
        return None;
    }
    let size = count(value);
    if size > *budget {
        return None;
    }
    *budget -= size;
    let mut out = Vec::with_capacity(value.len());
    for c in value {
        match c {
            Component::Func(name, inner) if name.eq_ignore_ascii_case("var") => {
                let args = value::split_commas_first(inner);
                let var_name = match args.0.iter().find(|c| !c.is_whitespace()) {
                    Some(Component::Tok(Tok::Ident(n))) => n.clone(),
                    _ => return None,
                };
                let replacement = match lookup(&var_name) {
                    Some(v) => substitute_budget(&v, lookup, depth + 1, budget)?,
                    None => {
                        let fallback = args.1?;
                        substitute_budget(&fallback, lookup, depth + 1, budget)?
                    }
                };
                out.extend(css::trim(&replacement));
            }
            Component::Func(name, inner) => {
                out.push(Component::Func(
                    name.clone(),
                    substitute_budget(inner, lookup, depth, budget)?,
                ));
            }
            Component::Paren(inner) => out.push(Component::Paren(substitute_budget(
                inner, lookup, depth, budget,
            )?)),
            Component::Square(inner) => out.push(Component::Square(substitute_budget(
                inner, lookup, depth, budget,
            )?)),
            other => out.push(other.clone()),
        }
    }
    Some(out)
}

#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub font_size: f64,
    pub root_font_size: f64,
    pub vw: f64,
    pub vh: f64,
    /// Mimic the browser's layout rounding (border widths snap to whole pixels). Only for
    /// comparing against captured computed style; rewrite equality is exact.
    pub layout_rounding: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Num {
    Length(f64),
    Scalar(f64),
}

fn calc_term(tokens: &[&Component], m: &Metrics) -> Option<Num> {
    // Evaluate a flat sequence of operands and * / + - operators.
    let mut ops: Vec<char> = Vec::new();
    let mut vals: Vec<Num> = Vec::new();
    let mut i = 0;
    let mut expect_value = true;
    while i < tokens.len() {
        let t = tokens[i];
        if expect_value {
            let v = match t {
                Component::Tok(Tok::Number { value, .. }) => Num::Scalar(f64::from(*value)),
                Component::Tok(Tok::Dimension { value, unit, .. }) => {
                    let ctx = Context {
                        rem_px: Some(m.root_font_size),
                    };
                    match unit.to_ascii_lowercase().as_str() {
                        "em" => Num::Length(f64::from(*value) * m.font_size),
                        "vw" => Num::Length(f64::from(*value) * m.vw / 100.0),
                        "vh" => Num::Length(f64::from(*value) * m.vh / 100.0),
                        _ => match value::parse_length(t, &ctx)? {
                            Length::Px(p) => Num::Length(p),
                            Length::Relative(..) => return None,
                        },
                    }
                }
                Component::Paren(inner) => eval_sum(inner, m)?,
                Component::Func(name, inner) if name.eq_ignore_ascii_case("calc") => {
                    eval_sum(inner, m)?
                }
                Component::Func(name, inner)
                    if matches!(name.to_ascii_lowercase().as_str(), "min" | "max") =>
                {
                    let args = value::split_commas(inner);
                    let mut nums = Vec::new();
                    for a in args {
                        nums.push(eval_sum(&a, m)?);
                    }
                    let lengths: Vec<f64> = nums
                        .iter()
                        .map(|n| match n {
                            Num::Length(v) | Num::Scalar(v) => *v,
                        })
                        .collect();
                    let is_len = matches!(nums.first(), Some(Num::Length(_)));
                    if nums.iter().any(|n| matches!(n, Num::Length(_)) != is_len) {
                        return None;
                    }
                    let v = if name.eq_ignore_ascii_case("min") {
                        lengths.iter().cloned().fold(f64::INFINITY, f64::min)
                    } else {
                        lengths.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
                    };
                    if is_len {
                        Num::Length(v)
                    } else {
                        Num::Scalar(v)
                    }
                }
                _ => return None,
            };
            vals.push(v);
            expect_value = false;
        } else {
            match t {
                Component::Tok(Tok::Delim(c @ ('*' | '/' | '+' | '-'))) => ops.push(*c),
                _ => return None,
            }
            expect_value = true;
        }
        i += 1;
    }
    if expect_value || vals.is_empty() {
        return None;
    }
    // First pass: * and /
    let mut values = vec![vals[0]];
    let mut adds: Vec<char> = Vec::new();
    for (op, rhs) in ops.iter().zip(vals.iter().skip(1)) {
        match op {
            '*' | '/' => {
                let lhs = values.pop()?;
                let r = match (lhs, *rhs, *op) {
                    (Num::Length(a), Num::Scalar(b), '*')
                    | (Num::Scalar(b), Num::Length(a), '*') => Num::Length(a * b),
                    (Num::Scalar(a), Num::Scalar(b), '*') => Num::Scalar(a * b),
                    (Num::Length(a), Num::Scalar(b), '/') if b != 0.0 => Num::Length(a / b),
                    (Num::Scalar(a), Num::Scalar(b), '/') if b != 0.0 => Num::Scalar(a / b),
                    _ => return None,
                };
                values.push(r);
            }
            other => {
                adds.push(*other);
                values.push(*rhs);
            }
        }
    }
    let mut acc = values[0];
    for (op, rhs) in adds.iter().zip(values.iter().skip(1)) {
        acc = match (acc, *rhs, op) {
            (Num::Length(a), Num::Length(b), '+') => Num::Length(a + b),
            (Num::Length(a), Num::Length(b), '-') => Num::Length(a - b),
            (Num::Scalar(a), Num::Scalar(b), '+') => Num::Scalar(a + b),
            (Num::Scalar(a), Num::Scalar(b), '-') => Num::Scalar(a - b),
            _ => return None,
        };
    }
    Some(acc)
}

fn eval_sum(components: &[Component], m: &Metrics) -> Option<Num> {
    // `+`/`-` inside calc() require surrounding whitespace, `*` and `/` do not; a "-" glued
    // to a number is a sign and already part of the number token.
    let tokens: Vec<&Component> = components.iter().filter(|c| !c.is_whitespace()).collect();
    calc_term(&tokens, m)
}

/// Evaluate a length expression to CSS pixels (numbers, dimensions, `calc()`, `min()`, `max()`).
pub fn to_px(value: &[Component], m: &Metrics) -> Option<f64> {
    let tokens: Vec<&Component> = value.iter().filter(|c| !c.is_whitespace()).collect();
    match calc_term(&tokens, m)? {
        Num::Length(v) => Some(v),
        Num::Scalar(0.0) => Some(0.0),
        Num::Scalar(_) => None,
    }
}

// ----- canonical forms ---------------------------------------------------------

pub fn is_color_property(name: &str) -> bool {
    matches!(
        name,
        "color"
            | "background-color"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "outline-color"
            | "text-decoration-color"
            | "caret-color"
            | "column-rule-color"
            | "accent-color"
            | "fill"
            | "stroke"
            | "stop-color"
            | "flood-color"
            | "lighting-color"
            | "-webkit-text-fill-color"
            | "-webkit-text-stroke-color"
            | "text-emphasis-color"
    )
}

fn length_property(name: &str) -> bool {
    matches!(
        name,
        "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "row-gap"
            | "column-gap"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
            | "outline-width"
            | "outline-offset"
            | "min-width"
            | "min-height"
            | "max-width"
            | "max-height"
            | "width"
            | "height"
            | "text-indent"
            | "flex-basis"
            | "scroll-margin-top"
            | "scroll-margin-right"
            | "scroll-margin-bottom"
            | "scroll-margin-left"
            | "scroll-padding-top"
            | "scroll-padding-right"
            | "scroll-padding-bottom"
            | "scroll-padding-left"
    )
}

fn radius_property(name: &str) -> bool {
    matches!(
        name,
        "border-top-left-radius"
            | "border-top-right-radius"
            | "border-bottom-right-radius"
            | "border-bottom-left-radius"
    )
}

pub fn comparable_property(name: &str) -> bool {
    is_color_property(name)
        || length_property(name)
        || radius_property(name)
        || matches!(
            name,
            "font-size"
                | "font-weight"
                | "font-family"
                | "letter-spacing"
                | "line-height"
                | "opacity"
                | "z-index"
                | "box-shadow"
                | "text-shadow"
                | "transition-duration"
                | "transition-delay"
                | "transition-timing-function"
                | "animation-duration"
                | "animation-delay"
                | "animation-timing-function"
                | "word-spacing"
        )
}

/// Canonical text for a color (computed or authored).
fn canon_color(components: &[Component]) -> Option<String> {
    let c = components.iter().find(|c| !c.is_whitespace())?;
    if components.iter().filter(|c| !c.is_whitespace()).count() != 1 {
        return None;
    }
    if c.ident()
        .is_some_and(|i| i.eq_ignore_ascii_case("currentcolor"))
    {
        return None;
    }
    match value::parse_color(c)? {
        Color::Srgb(rgba) => Some(Color::Srgb(rgba).key()),
        Color::Other(_) => Some(
            canon_modern_color(c)
                .unwrap_or_else(|| value::parse_color(c).map(|c| c.key()).unwrap_or_default()),
        ),
    }
}

/// `oklch(94.3% .016 250)` and the computed `oklch(0.943 0.016 250)` share a canonical form.
fn canon_modern_color(c: &Component) -> Option<String> {
    let Component::Func(name, inner) = c else {
        return None;
    };
    let lower = name.to_ascii_lowercase();
    if !matches!(lower.as_str(), "oklch" | "oklab") {
        return None;
    }
    let mut numbers: Vec<f64> = Vec::new();
    let mut alpha = 1.0;
    let mut in_alpha = false;
    for part in inner.iter().filter(|c| !c.is_whitespace()) {
        match part {
            Component::Tok(Tok::Delim('/')) => in_alpha = true,
            Component::Tok(Tok::Number { value, .. }) => {
                if in_alpha {
                    alpha = f64::from(*value)
                } else {
                    numbers.push(f64::from(*value))
                }
            }
            Component::Tok(Tok::Percentage { unit_value, .. }) => {
                let v = f64::from(*unit_value);
                if in_alpha {
                    alpha = v;
                } else if numbers.is_empty() {
                    numbers.push(v); // lightness: 100% = 1
                } else if lower == "oklch" && numbers.len() == 1 {
                    numbers.push(v * 0.4); // chroma: 100% = 0.4
                } else {
                    numbers.push(v * 0.4); // oklab a/b
                }
            }
            Component::Tok(Tok::Dimension { value, unit, .. })
                if lower == "oklch" && numbers.len() == 2 =>
            {
                let v = f64::from(*value);
                numbers.push(match unit.to_ascii_lowercase().as_str() {
                    "deg" => v,
                    "turn" => v * 360.0,
                    "rad" => v.to_degrees(),
                    "grad" => v * 0.9,
                    _ => return None,
                });
            }
            Component::Tok(Tok::Ident(i)) if i.eq_ignore_ascii_case("none") => numbers.push(0.0),
            _ => return None,
        }
    }
    if numbers.len() != 3 {
        return None;
    }
    let l = (numbers[0] * 1e4).round() / 1e4;
    let (b, c3) = (numbers[1], numbers[2]);
    let r = |v: f64| fmt_num((v * 1e3).round() / 1e3);
    Some(format!(
        "other:{lower}({} {} {} / {})",
        r(l),
        r(b),
        r(c3),
        r(alpha)
    ))
}

fn px_text(v: f64) -> String {
    format!("{}px", fmt_num(v))
}

fn weight(components: &[Component]) -> Option<String> {
    value::font_weight_key(components)
}

/// Canonical comparable text for `name: value`, or `None` if not comparable.
pub fn canonical(name: &str, components: &[Component], m: &Metrics) -> Option<String> {
    let trimmed = css::trim(components);
    if trimmed.is_empty() {
        return None;
    }
    if is_color_property(name) {
        return canon_color(&trimmed);
    }
    let keyword = match trimmed.as_slice() {
        [Component::Tok(Tok::Ident(i))] => Some(i.to_ascii_lowercase()),
        _ => None,
    };
    if let Some(k) = &keyword
        && matches!(
            k.as_str(),
            "inherit" | "initial" | "unset" | "revert" | "revert-layer"
        )
    {
        return None;
    }
    if length_property(name) {
        if let Some(k) = keyword {
            return Some(k);
        }
        if trimmed
            .iter()
            .any(|c| matches!(c, Component::Tok(Tok::Percentage { .. })))
        {
            return None;
        }
        let px = to_px(&trimmed, m)?;
        // Border widths snap down to whole device pixels (minimum 1px) at 1x.
        if m.layout_rounding && name.starts_with("border-") && name.ends_with("-width") && px > 0.0
        {
            return Some(px_text(px.floor().max(1.0)));
        }
        return Some(px_text(px));
    }
    if radius_property(name) {
        let parts = parts(&trimmed);
        let mut out = Vec::new();
        for p in parts {
            if matches!(p.as_slice(), [Component::Tok(Tok::Percentage { .. })]) {
                return None;
            }
            out.push(px_text(to_px(&p, m)?));
        }
        if out.len() == 2 && out[0] == out[1] {
            out.pop();
        }
        return if out.is_empty() {
            None
        } else {
            Some(out.join(" "))
        };
    }
    match name {
        "font-size" => {
            if keyword.is_some() {
                return None;
            }
            if let [Component::Tok(Tok::Percentage { unit_value, .. })] = trimmed.as_slice() {
                return Some(px_text(f64::from(*unit_value) * m.font_size)); // m.font_size = parent's here
            }
            to_px(&trimmed, m).map(px_text)
        }
        "letter-spacing" | "word-spacing" => {
            if let Some(k) = keyword {
                return Some(if k == "normal" { px_text(0.0) } else { k });
            }
            to_px(&trimmed, m).map(px_text)
        }
        "line-height" => {
            if let Some(k) = keyword {
                return Some(k);
            }
            match trimmed.as_slice() {
                [Component::Tok(Tok::Number { value, .. })] => {
                    Some(px_text(f64::from(*value) * m.font_size))
                }
                [Component::Tok(Tok::Percentage { unit_value, .. })] => {
                    Some(px_text(f64::from(*unit_value) * m.font_size))
                }
                _ => to_px(&trimmed, m).map(px_text),
            }
        }
        "font-weight" => weight(&trimmed),
        "font-family" => value::font_family_key(&trimmed),
        "opacity" => match trimmed.as_slice() {
            [Component::Tok(Tok::Number { value, .. })] => Some(fmt_num(f64::from(*value))),
            [Component::Tok(Tok::Percentage { unit_value, .. })] => {
                Some(fmt_num(f64::from(*unit_value)))
            }
            _ => None,
        },
        "z-index" => match (keyword, value::parse_int(&trimmed)) {
            (Some(k), _) => Some(k),
            (None, Some(v)) => Some(v.to_string()),
            _ => None,
        },
        "box-shadow" | "text-shadow" => {
            if keyword.as_deref() == Some("none") {
                return Some("none".into());
            }
            let ctx = Context {
                rem_px: Some(m.root_font_size),
            };
            let text = name == "text-shadow";
            let layers = value::parse_shadow(&trimmed, &ctx, text)?;
            // Shadow lengths in em/% cannot be compared without a font context here.
            if layers
                .iter()
                .any(|l| l.lengths.iter().any(|x| x.px().is_none()))
            {
                return None;
            }
            Some(value::shadow_key(&layers, text))
        }
        "transition-duration" | "transition-delay" | "animation-duration" | "animation-delay" => {
            let mut out = Vec::new();
            for part in value::split_commas(&trimmed) {
                match part.as_slice() {
                    [c] => out.push(fmt_num(value::parse_time(c)?)),
                    _ => return None,
                }
            }
            Some(out.join(","))
        }
        "transition-timing-function" | "animation-timing-function" => {
            let mut out = Vec::new();
            for part in value::split_commas(&trimmed) {
                match part.as_slice() {
                    [c @ Component::Func(..)] => out.push(value::easing_key(c)?),
                    [Component::Tok(Tok::Ident(i))] => out.push(i.to_ascii_lowercase()),
                    _ => return None,
                }
            }
            Some(out.join(","))
        }
        _ => None,
    }
}

/// Equality for canonical forms: exact, or equal within layout rounding (1/64 px) when both
/// are lists of pixel lengths.
pub fn canonical_eq(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let pa: Option<Vec<f64>> = a
        .split(' ')
        .map(|p| p.strip_suffix("px")?.parse().ok())
        .collect();
    let pb: Option<Vec<f64>> = b
        .split(' ')
        .map(|p| p.strip_suffix("px")?.parse().ok())
        .collect();
    match (pa, pb) {
        (Some(x), Some(y)) => {
            x.len() == y.len() && x.iter().zip(&y).all(|(p, q)| (p - q).abs() <= 0.02)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::css::parse_components;
    use super::*;

    fn m() -> Metrics {
        Metrics {
            font_size: 16.0,
            root_font_size: 16.0,
            vw: 1440.0,
            vh: 900.0,
            layout_rounding: true,
        }
    }
    fn canon(name: &str, text: &str) -> Option<String> {
        canonical(name, &parse_components(text), &m())
    }

    #[test]
    fn calc_and_spacing_scale_evaluate_to_pixels() {
        let spacing = |v: &str| {
            let lookup = |n: &str| (n == "--spacing").then(|| parse_components("0.25rem"));
            substitute_vars(&parse_components(v), &lookup, 0)
        };
        let v = spacing("calc(var(--spacing) * 4)").unwrap();
        assert_eq!(to_px(&v, &m()), Some(16.0));
        let v = spacing("calc(var(--spacing) * -1.5)").unwrap();
        assert_eq!(to_px(&v, &m()), Some(-6.0));
        let v = spacing("calc(100vw - 2 * var(--spacing))").unwrap();
        assert_eq!(to_px(&v, &m()), Some(1432.0));
        assert!(spacing("var(--missing)").is_none());
        let fallback =
            substitute_vars(&parse_components("var(--missing, 3px)"), &|_| None, 0).unwrap();
        assert_eq!(to_px(&fallback, &m()), Some(3.0));
    }

    #[test]
    fn canonical_forms_agree_between_authored_and_computed() {
        assert_eq!(
            canon("color", "#ffffff"),
            canon("color", "rgb(255, 255, 255)")
        );
        assert_eq!(
            canon("color", "oklch(94.3% .016 250)"),
            canon("color", "oklch(0.943 0.016 250)")
        );
        assert_eq!(canon("padding-top", "1rem"), canon("padding-top", "16px"));
        assert_eq!(canon("line-height", "1.5"), canon("line-height", "24px"));
        assert_eq!(canon("opacity", "0.5"), canon("opacity", "50%"));
        assert_eq!(
            canon("transition-duration", ".2s"),
            canon("transition-duration", "200ms")
        );
        assert_eq!(
            canon("box-shadow", "0 1px 2px rgba(0,0,0,.5)"),
            canon("box-shadow", "rgba(0, 0, 0, 0.5) 0px 1px 2px 0px")
        );
        assert_eq!(canon("font-weight", "bold"), canon("font-weight", "700"));
        assert!(canon("width", "50%").is_none());
        assert!(canon("color", "currentcolor").is_none());
    }

    #[test]
    fn shorthands_expand_or_become_opaque() {
        let Expansion::Longhands(l) = expand("margin", &parse_components("1px 2px")) else {
            panic!()
        };
        assert_eq!(l.len(), 4);
        assert_eq!(l[1].0, "margin-right");
        assert_eq!(css::to_css(&l[3].1), "2px");
        let Expansion::Longhands(b) = expand("border", &parse_components("1px solid #fff")) else {
            panic!()
        };
        assert_eq!(b.len(), 12);
        assert_eq!(b[0].0, "border-top-width");
        assert!(
            matches!(expand("background", &parse_components("red url(x)")), Expansion::Opaque(l) if l.contains(&"background-color"))
        );
        assert!(matches!(
            expand("margin", &parse_components("var(--x)")),
            Expansion::Opaque(_)
        ));
        assert_eq!(expand("color", &parse_components("red")), Expansion::Not);
        let Expansion::Longhands(r) = expand("border-radius", &parse_components("4px 8px")) else {
            panic!()
        };
        assert_eq!(css::to_css(&r[2].1), "4px");
    }
}
