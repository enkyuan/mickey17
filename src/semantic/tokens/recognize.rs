//! Recognition of tokenizable values inside declarations.
//!
//! One walker serves both the inventory (collect hits) and the rewriter (replace hits),
//! so what is counted is exactly what can be replaced.

use super::css::{Component, Tok, trim};
use super::value::{self, Category, Color, Context, Length, fmt_num};

#[derive(Debug, Clone)]
pub struct Hit {
    pub category: Category,
    /// Unit-normalized merge key (unique per category).
    pub key: String,
    /// The value as the site wrote it.
    pub spelling: String,
    pub property: String,
    /// Normalized length when the hit is a length (spacing, radius, font-size, ...).
    pub length: Option<Length>,
    /// Authored unit family (`px`, `rem`, ...), for merge reporting.
    pub unit: Option<String>,
}

/// Properties whose values may contain colors that are safe to tokenize.
fn color_context(property: &str) -> bool {
    matches!(
        property,
        "color"
            | "background-color"
            | "background"
            | "background-image"
            | "border"
            | "border-color"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "outline"
            | "outline-color"
            | "text-decoration"
            | "text-decoration-color"
            | "box-shadow"
            | "text-shadow"
            | "fill"
            | "stroke"
            | "caret-color"
            | "accent-color"
            | "column-rule"
            | "column-rule-color"
            | "stop-color"
            | "flood-color"
            | "scrollbar-color"
            | "-webkit-text-fill-color"
            | "-webkit-text-stroke"
            | "-webkit-text-stroke-color"
            | "text-emphasis-color"
            | "mask-image"
            | "border-image-source"
            | "filter"
    )
}

fn spacing_context(property: &str) -> bool {
    matches!(
        property,
        "margin"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "padding"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "gap"
            | "row-gap"
            | "column-gap"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "inset"
            | "width"
            | "height"
            | "min-width"
            | "min-height"
            | "max-width"
            | "max-height"
    )
}

fn radius_context(property: &str) -> bool {
    matches!(
        property,
        "border-radius"
            | "border-top-left-radius"
            | "border-top-right-radius"
            | "border-bottom-right-radius"
            | "border-bottom-left-radius"
    )
}

fn border_width_context(property: &str) -> bool {
    matches!(
        property,
        "border-width"
            | "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
    )
}

fn border_shorthand(property: &str) -> bool {
    matches!(
        property,
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left"
    )
}

fn unit_of(component: &Component) -> Option<String> {
    match component {
        Component::Tok(Tok::Dimension { unit, .. }) => Some(unit.to_ascii_lowercase()),
        _ => None,
    }
}

fn nonzero_length(length: &Length) -> bool {
    match length {
        Length::Px(v) => v.abs() > 1e-9,
        Length::Relative(v, _) => v.abs() > 1e-9,
    }
}

fn is_color_function(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "rgb"
            | "rgba"
            | "hsl"
            | "hsla"
            | "oklch"
            | "oklab"
            | "lab"
            | "lch"
            | "hwb"
            | "color"
            | "color-mix"
            | "light-dark"
    )
}

type Replace<'a> = dyn FnMut(&Hit) -> Option<Vec<Component>> + 'a;

fn visit_colors(property: &str, value: &[Component], f: &mut Replace<'_>) -> Vec<Component> {
    let mut out = Vec::with_capacity(value.len());
    for c in value {
        if let Some(color) = value::parse_color(c) {
            let transparent = matches!(color, Color::Srgb([_, _, _, 0]));
            if !transparent {
                let hit = Hit {
                    category: Category::Color,
                    key: color.key(),
                    spelling: super::css::to_css(std::slice::from_ref(c)),
                    property: property.to_string(),
                    length: None,
                    unit: None,
                };
                if let Some(replacement) = f(&hit) {
                    out.extend(replacement);
                    continue;
                }
            }
            out.push(c.clone());
            continue;
        }
        match c {
            Component::Func(name, inner)
                if !is_color_function(name)
                    && !name.eq_ignore_ascii_case("var")
                    && !name.eq_ignore_ascii_case("url") =>
            {
                out.push(Component::Func(
                    name.clone(),
                    visit_colors(property, inner, f),
                ));
            }
            Component::Paren(inner) => out.push(Component::Paren(visit_colors(property, inner, f))),
            other => out.push(other.clone()),
        }
    }
    out
}

fn length_hit(category: Category, property: &str, c: &Component, ctx: &Context) -> Option<Hit> {
    let length = value::parse_length(c, ctx)?;
    if !nonzero_length(&length) {
        return None;
    }
    Some(Hit {
        category,
        key: length.key(),
        spelling: super::css::to_css(std::slice::from_ref(c)),
        property: property.to_string(),
        unit: unit_of(c),
        length: Some(length),
    })
}

/// Walk `value` of `property`, offering every tokenizable piece to `f`. When `f` returns a
/// replacement the piece is substituted; the (possibly rewritten) value is returned.
pub fn visit_value(
    property: &str,
    value: &[Component],
    ctx: &Context,
    f: &mut Replace<'_>,
) -> Vec<Component> {
    // Values that depend on var()/env()/attr() at the top level are never touched.
    let trimmed = trim(value);
    if trimmed.is_empty() {
        return value.to_vec();
    }
    let mut changed = false;

    let whole = |category: Category,
                 key: String,
                 spelling: String,
                 f: &mut Replace<'_>|
     -> Option<Vec<Component>> {
        let hit = Hit {
            category,
            key,
            spelling,
            property: property.to_string(),
            length: None,
            unit: None,
        };
        f(&hit)
    };

    match property {
        "font-family" => {
            if let Some(key) = value::font_family_key(&trimmed)
                && let Some(rep) = whole(Category::FontFamily, key, super::css::to_css(&trimmed), f)
            {
                return rep;
            }
            return value.to_vec();
        }
        "box-shadow" | "text-shadow" => {
            let text = property == "text-shadow";
            if let Some(layers) = value::parse_shadow(&trimmed, ctx, text) {
                // Only fully literal shadows; relative (em/%) lengths stay as written.
                if layers
                    .iter()
                    .all(|l| l.lengths.iter().all(|x| x.px().is_some()))
                {
                    let key = value::shadow_key(&layers, text);
                    if let Some(rep) = whole(Category::Shadow, key, super::css::to_css(&trimmed), f)
                    {
                        return rep;
                    }
                }
            }
            return value.to_vec();
        }
        "font-weight" => {
            if let [Component::Tok(Tok::Number { value: v, .. })] = trimmed.as_slice() {
                let key = fmt_num(f64::from(*v));
                if let Some(rep) = whole(Category::FontWeight, key, super::css::to_css(&trimmed), f)
                {
                    return rep;
                }
            }
            return value.to_vec();
        }
        "z-index" => {
            if let Some(n) = value::parse_int(&trimmed)
                && let Some(rep) = whole(Category::ZIndex, n.to_string(), n.to_string(), f)
            {
                return rep;
            }
            return value.to_vec();
        }
        "line-height" => {
            let hit = match trimmed.as_slice() {
                [Component::Tok(Tok::Number { value: v, .. })] => {
                    Some((fmt_num(f64::from(*v)), super::css::to_css(&trimmed)))
                }
                [c @ Component::Tok(Tok::Dimension { .. })] => value::parse_length(c, ctx)
                    .filter(nonzero_length)
                    .map(|l| (l.key(), super::css::to_css(std::slice::from_ref(c)))),
                _ => None,
            };
            if let Some((key, spelling)) = hit
                && let Some(rep) = whole(Category::LineHeight, key, spelling, f)
            {
                return rep;
            }
            return value.to_vec();
        }
        _ => {}
    }

    // Component-wise categories.
    let single_length_category = match property {
        "font-size" => Some(Category::FontSize),
        "letter-spacing" => Some(Category::LetterSpacing),
        p if radius_context(p) => Some(Category::Radius),
        p if border_width_context(p) => Some(Category::BorderWidth),
        p if spacing_context(p) => Some(Category::Spacing),
        _ => None,
    };
    if let Some(category) = single_length_category {
        let mut out = Vec::with_capacity(trimmed.len());
        for c in &trimmed {
            if let Some(hit) = length_hit(category, property, c, ctx) {
                // Percentages and viewport units are never spacing-scale members.
                let scalable = category != Category::Spacing
                    || hit.length.as_ref().and_then(Length::px).is_some();
                if scalable && let Some(rep) = f(&hit) {
                    out.extend(rep);
                    changed = true;
                    continue;
                }
            }
            out.push(c.clone());
        }
        return if changed { out } else { value.to_vec() };
    }

    match property {
        "transition-duration"
        | "transition-delay"
        | "animation-duration"
        | "animation-delay"
        | "transition"
        | "animation"
        | "transition-timing-function"
        | "animation-timing-function" => {
            let mut out = Vec::with_capacity(trimmed.len());
            for c in &trimmed {
                if let Some(ms) = value::parse_time(c) {
                    if ms > 0.0 {
                        let hit = Hit {
                            category: Category::Duration,
                            key: fmt_num(ms),
                            spelling: super::css::to_css(std::slice::from_ref(c)),
                            property: property.to_string(),
                            length: None,
                            unit: unit_of(c),
                        };
                        if let Some(rep) = f(&hit) {
                            out.extend(rep);
                            changed = true;
                            continue;
                        }
                    }
                } else if let Some(key) = value::easing_key(c) {
                    let hit = Hit {
                        category: Category::Easing,
                        key,
                        spelling: super::css::to_css(std::slice::from_ref(c)),
                        property: property.to_string(),
                        length: None,
                        unit: None,
                    };
                    if let Some(rep) = f(&hit) {
                        out.extend(rep);
                        changed = true;
                        continue;
                    }
                }
                out.push(c.clone());
            }
            return if changed { out } else { value.to_vec() };
        }
        _ => {}
    }

    if border_shorthand(property) {
        let mut out = Vec::with_capacity(trimmed.len());
        for c in &trimmed {
            if let Some(hit) = length_hit(Category::BorderWidth, property, c, ctx)
                && let Some(rep) = f(&hit)
            {
                out.extend(rep);
                changed = true;
                continue;
            }
            out.push(c.clone());
        }
        let out = if changed { out } else { value.to_vec() };
        if color_context(property) {
            return visit_colors(property, &out, f);
        }
        return out;
    }

    if color_context(property) {
        let out = visit_colors(property, &trimmed, f);
        return if out == trimmed { value.to_vec() } else { out };
    }
    value.to_vec()
}

#[cfg(test)]
mod tests {
    use super::super::css::{parse_components, to_css};
    use super::*;

    fn collect(property: &str, text: &str) -> Vec<(Category, String)> {
        let ctx = Context { rem_px: Some(16.0) };
        let mut hits = Vec::new();
        visit_value(property, &parse_components(text), &ctx, &mut |h| {
            hits.push((h.category, h.key.clone()));
            None
        });
        hits
    }

    #[test]
    fn finds_colors_inside_shorthands_and_gradients() {
        let hits = collect("border", "1px solid #fff");
        assert!(hits.contains(&(Category::BorderWidth, "px:1".into())));
        assert!(hits.iter().any(|(c, _)| *c == Category::Color));
        let hits = collect(
            "background-image",
            "linear-gradient(90deg, #000 0%, rgba(0,0,0,0) 100%)",
        );
        assert_eq!(
            hits.iter().filter(|(c, _)| *c == Category::Color).count(),
            1,
            "transparent stays a keyword"
        );
    }

    #[test]
    fn spacing_skips_zero_percent_and_viewport_units() {
        let hits = collect("padding", "0 16px 10% 2vw");
        assert_eq!(hits, vec![(Category::Spacing, "px:16".into())]);
    }

    #[test]
    fn replacement_substitutes_in_place_and_leaves_the_rest() {
        let ctx = Context { rem_px: Some(16.0) };
        let mut f = |h: &Hit| {
            (h.category == Category::Spacing && h.key == "px:16")
                .then(|| parse_components("var(--x)"))
        };
        let out = visit_value(
            "padding",
            &parse_components("0 16px 8px 16px"),
            &ctx,
            &mut f,
        );
        assert_eq!(to_css(&out), "0 var(--x) 8px var(--x)");
        let mut g = |h: &Hit| (h.category == Category::Color).then(|| parse_components("var(--c)"));
        let out = visit_value(
            "box-shadow",
            &parse_components("0 1px 2px #000"),
            &ctx,
            &mut g,
        );
        assert_eq!(
            to_css(&out),
            "0 1px 2px #000",
            "shadows are whole-value tokens, not per-color"
        );
    }

    #[test]
    fn whole_value_categories() {
        assert_eq!(
            collect("font-family", "\"STK Gerhard\", sans-serif")[0].0,
            Category::FontFamily
        );
        assert_eq!(
            collect("font-weight", "360")[0],
            (Category::FontWeight, "360".into())
        );
        assert!(collect("font-weight", "bold").is_empty());
        assert_eq!(collect("z-index", "40")[0], (Category::ZIndex, "40".into()));
        assert_eq!(
            collect("line-height", "1.5")[0],
            (Category::LineHeight, "1.5".into())
        );
        let t = collect("transition", "opacity .2s cubic-bezier(.4,0,.2,1) 0s");
        assert_eq!(t.len(), 2);
        assert!(
            t.iter()
                .any(|(c, k)| *c == Category::Duration && k == "200")
        );
        assert!(collect("margin", "var(--x)").is_empty());
    }
}
