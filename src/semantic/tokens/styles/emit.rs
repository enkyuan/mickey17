//! Placements -> class lists, and plain-CSS recipes for what utilities cannot express
//! unambiguously (overlapping shorthands, pseudo-elements, unsafe values).

use super::super::css;
use super::classes::{self, Theme};
use super::layers::{Layer, Placed, REVERT, State, overlaps};
use std::collections::BTreeMap;

pub fn variant_prefix(min: Option<u32>, state: Option<State>) -> String {
    let mut out = String::new();
    if let Some(w) = min {
        out.push_str(&format!("min-[{w}px]:"));
    }
    if let Some(s) = state {
        out.push_str(s.variant());
        out.push(':');
    }
    out
}

/// Four-sided families that collapse into their shorthand when all sides agree.
const SIDES: &[(&str, [&str; 4])] = &[
    (
        "padding",
        [
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
        ],
    ),
    (
        "margin",
        ["margin-top", "margin-right", "margin-bottom", "margin-left"],
    ),
    ("inset", ["top", "right", "bottom", "left"]),
    (
        "border-width",
        [
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
        ],
    ),
    (
        "border-style",
        [
            "border-top-style",
            "border-right-style",
            "border-bottom-style",
            "border-left-style",
        ],
    ),
    (
        "border-color",
        [
            "border-top-color",
            "border-right-color",
            "border-bottom-color",
            "border-left-color",
        ],
    ),
    (
        "border-radius",
        [
            "border-top-left-radius",
            "border-top-right-radius",
            "border-bottom-right-radius",
            "border-bottom-left-radius",
        ],
    ),
];

fn render_one(prop: &str, value: &Option<String>, theme: &Theme) -> Option<String> {
    match value {
        None => classes::render(prop, &css::parse_components(REVERT), theme)
            .filter(|c| c.starts_with('[')),
        Some(v) => classes::render(prop, &css::parse_components(v), theme),
    }
}

/// Utilities for a set of placements, sorted deterministically. `None` when some value has no
/// safe class spelling (the element then goes to a plain-CSS recipe).
pub fn render_classes(placed: &[Placed], theme: &Theme) -> Option<Vec<String>> {
    type Variant = (Option<u32>, Option<State>);
    let mut groups: BTreeMap<Variant, BTreeMap<&str, &Option<String>>> = BTreeMap::new();
    for p in placed {
        groups
            .entry((p.min, p.state))
            .or_default()
            .insert(p.prop.as_str(), &p.value);
    }
    let mut out = Vec::new();
    for ((min, state), mut props) in groups {
        let prefix = variant_prefix(min, state);
        for (short, sides) in SIDES {
            let first: Option<&Option<String>> = props.get(sides[0]).copied();
            let all_equal = first.is_some_and(|f| {
                f.is_some() && sides.iter().all(|s| props.get(s).copied() == Some(f))
            });
            if all_equal {
                let first = first.expect("checked");
                for s in sides {
                    props.remove(s);
                }
                props.insert(short, first);
            }
        }
        for (prop, value) in props {
            out.push(format!("{prefix}{}", render_one(prop, value, theme)?));
        }
    }
    Some(out)
}

fn block(selector: &str, decls: &[(String, String)], indent: &str) -> String {
    let mut out = format!("{indent}{selector} {{\n");
    for (p, v) in decls {
        out.push_str(&format!("{indent}  {p}: {v};\n"));
    }
    out.push_str(&format!("{indent}}}\n"));
    out
}

/// What a block has to write on top of what is already in force (`prev`): `revert` for what
/// disappeared, then the changed declarations in precedence order. A declaration that did not
/// change is written again when an earlier one in the block overlaps it (a shorthand resets its
/// longhands), so the cascade inside the block ends where the layer does.
fn delta(
    order: &[String],
    decls: &BTreeMap<String, String>,
    prev: &BTreeMap<String, String>,
    prev_order: &[String],
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // Two overlapping declarations (`background` / `background-color`) that swapped places keep
    // their values but change who wins: write the whole layer.
    let position = |list: &[String], p: &str| list.iter().position(|x| x == p);
    let reordered = !prev_order.is_empty()
        && order.iter().enumerate().any(|(i, a)| {
            order[i + 1..].iter().any(|b| {
                overlaps(a, b)
                    && matches!(
                        (position(prev_order, a), position(prev_order, b)),
                        (Some(x), Some(y)) if x > y
                    )
            })
        });
    if reordered {
        return prev
            .keys()
            .filter(|p| !decls.contains_key(*p))
            .map(|p| (p.clone(), REVERT.to_string()))
            .chain(
                order
                    .iter()
                    .filter_map(|p| decls.get(p).map(|v| (p.clone(), v.clone()))),
            )
            .collect();
    }
    let mut written: Vec<&String> = Vec::new();
    for p in prev.keys() {
        if !decls.contains_key(p) {
            out.push((p.clone(), REVERT.to_string()));
            written.push(p);
        }
    }
    for p in order {
        let Some(v) = decls.get(p) else { continue };
        if prev.get(p) != Some(v) || written.iter().any(|q| overlaps(q, p)) {
            out.push((p.clone(), v.clone()));
            written.push(p);
        }
    }
    out
}

fn ordered(layer: &Layer, previous: Option<&Layer>) -> Vec<(String, String)> {
    let none = BTreeMap::new();
    delta(
        &layer.order,
        &layer.decls,
        previous.map_or(&none, |p| &p.decls),
        previous.map_or(&[][..], |p| &p.order),
    )
}

fn wrap(min: Option<u32>, inner: String) -> String {
    match min {
        None => inner,
        Some(w) => {
            let indented: String = inner.lines().map(|l| format!("  {l}\n")).collect();
            format!("@media (width >= {w}px) {{\n{indented}}}\n")
        }
    }
}

/// Plain CSS for an element whose declarations cannot be ordered as utilities: every width
/// interval as a full layer, then each forced state as the declarations that differ.
pub fn recipe_css(
    name: &str,
    bounds: &[u32],
    rest: &[Layer],
    states: &BTreeMap<State, Vec<Layer>>,
) -> String {
    let selector = format!(".{name}");
    let mut out = String::new();
    for (i, layer) in rest.iter().enumerate() {
        let decls = ordered(layer, if i == 0 { None } else { Some(&rest[i - 1]) });
        if decls.is_empty() {
            continue;
        }
        out.push_str(&wrap(
            (i > 0).then(|| bounds[i]),
            block(&selector, &decls, ""),
        ));
    }
    let mut carried: BTreeMap<State, BTreeMap<String, Option<String>>> = BTreeMap::new();
    for state in State::ALL {
        let Some(layers) = states.get(&state) else {
            continue;
        };
        for (i, layer) in layers.iter().enumerate() {
            // What is in force when this state applies: the state it builds on (its full layer at
            // this interval) or the plain layer, plus what earlier intervals of this state carried.
            let mut effective = match state.parent().and_then(|p| states.get(&p)) {
                Some(parent) => parent[i].decls.clone(),
                None => rest[i].decls.clone(),
            };
            for (p, v) in carried.get(&state).into_iter().flatten() {
                match v {
                    Some(v) => {
                        effective.insert(p.clone(), v.clone());
                    }
                    None => {
                        effective.remove(p);
                    }
                }
            }
            let decls = delta(&layer.order, &layer.decls, &effective, &[]);
            let entry = carried.entry(state).or_default();
            for (p, v) in &decls {
                entry.insert(p.clone(), (v != REVERT).then(|| v.clone()));
            }
            if decls.is_empty() {
                continue;
            }
            let rule = if state == State::ReducedMotion {
                let inner = block(&selector, &decls, "");
                let indented: String = inner.lines().map(|l| format!("  {l}\n")).collect();
                format!("@media (prefers-reduced-motion: reduce) {{\n{indented}}}\n")
            } else {
                let rule = block(&format!("{selector}:{}", state.variant()), &decls, "");
                if state == State::Hover {
                    let indented: String = rule.lines().map(|l| format!("  {l}\n")).collect();
                    format!("@media (hover: hover) {{\n{indented}}}\n")
                } else {
                    rule
                }
            };
            out.push_str(&wrap((i > 0).then(|| bounds[i]), rule));
        }
    }
    out
}

/// Plain CSS for a pseudo-element of an element (`::before`, `::after`).
pub fn pseudo_css(name: &str, pseudo: &str, bounds: &[u32], layers: &[Layer]) -> String {
    let selector = format!(".{name}::{pseudo}");
    let mut out = String::new();
    for (i, layer) in layers.iter().enumerate() {
        let decls = ordered(layer, if i == 0 { None } else { Some(&layers[i - 1]) });
        if decls.is_empty() {
            continue;
        }
        out.push_str(&wrap(
            (i > 0).then(|| bounds[i]),
            block(&selector, &decls, ""),
        ));
    }
    out
}

/// A recipe made of utilities: `.name { @apply a b c; }`, one `@apply` per variant group so
/// long lists stay readable.
pub fn apply_css(name: &str, classes: &[String]) -> String {
    let mut out = format!(".{name} {{\n");
    for chunk in classes.chunks(6) {
        out.push_str(&format!("  @apply {};\n", chunk.join(" ")));
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::super::layers::place;
    use super::*;

    fn theme() -> Theme {
        let names = ["--color-ink".to_string()];
        Theme::from_definitions(names.iter(), Some(4.0))
    }

    fn layer(pairs: &[(&str, &str)]) -> Layer {
        Layer {
            decls: pairs
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            order: pairs.iter().map(|(a, _)| a.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn classes_carry_variants_and_collapse_equal_sides() {
        let rest = vec![
            layer(&[
                ("padding-top", "calc(var(--spacing) * 4)"),
                ("padding-right", "calc(var(--spacing) * 4)"),
                ("padding-bottom", "calc(var(--spacing) * 4)"),
                ("padding-left", "calc(var(--spacing) * 4)"),
                ("display", "none"),
                ("color", "var(--color-ink)"),
            ]),
            layer(&[
                ("padding-top", "calc(var(--spacing) * 8)"),
                ("padding-right", "calc(var(--spacing) * 4)"),
                ("padding-bottom", "calc(var(--spacing) * 4)"),
                ("padding-left", "calc(var(--spacing) * 4)"),
                ("color", "var(--color-ink)"),
            ]),
        ];
        let mut states = BTreeMap::new();
        states.insert(
            State::Hover,
            vec![
                layer(&[
                    ("display", "none"),
                    ("color", "red"),
                    ("padding-top", "calc(var(--spacing) * 4)"),
                    ("padding-right", "calc(var(--spacing) * 4)"),
                    ("padding-bottom", "calc(var(--spacing) * 4)"),
                    ("padding-left", "calc(var(--spacing) * 4)"),
                ]),
                layer(&[
                    ("padding-top", "calc(var(--spacing) * 8)"),
                    ("padding-right", "calc(var(--spacing) * 4)"),
                    ("padding-bottom", "calc(var(--spacing) * 4)"),
                    ("padding-left", "calc(var(--spacing) * 4)"),
                    ("color", "red"),
                ]),
            ],
        );
        let placed = place(&[0, 600], &rest, &states).placed;
        let classes = render_classes(&placed, &theme()).unwrap();
        assert!(classes.contains(&"p-4".to_string()), "{classes:?}");
        assert!(classes.contains(&"hidden".to_string()));
        assert!(classes.contains(&"text-ink".to_string()));
        assert!(
            classes.contains(&"min-[600px]:pt-8".to_string()),
            "{classes:?}"
        );
        assert!(
            classes.contains(&"min-[600px]:[display:revert]".to_string()),
            "{classes:?}"
        );
        assert!(
            classes.contains(&"hover:[color:red]".to_string()),
            "{classes:?}"
        );
    }

    #[test]
    fn unsafe_values_have_no_class_and_recipes_keep_order_and_reverts() {
        let placed = vec![Placed {
            min: None,
            state: None,
            prop: "background".into(),
            value: Some("url(a]b.png)".into()),
        }];
        assert!(render_classes(&placed, &theme()).is_none());
        let rest = vec![
            layer(&[("background", "red"), ("background-color", "blue")]),
            layer(&[("background", "green")]),
        ];
        let css = recipe_css("hero", &[0, 600], &rest, &BTreeMap::new());
        assert!(
            css.starts_with(".hero {\n  background: red;\n  background-color: blue;\n}"),
            "{css}"
        );
        assert!(css.contains("@media (width >= 600px) {\n  .hero {\n    background-color: revert;\n    background: green;\n  }\n}"), "{css}");
    }

    #[test]
    fn apply_recipes_group_utilities() {
        let css = apply_css(
            "nav-link",
            &["flex".into(), "p-4".into(), "min-[600px]:p-8".into()],
        );
        assert_eq!(css, ".nav-link {\n  @apply flex p-4 min-[600px]:p-8;\n}\n");
    }
}
