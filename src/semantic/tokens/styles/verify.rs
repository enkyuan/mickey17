//! Equality proof for a translation: the author declarations of every element, at every
//! width interval and forced state, resolved from the original (tokenized) stylesheets must be
//! the same as those resolved from the translated output (compiled utilities + recipes) with
//! the elements carrying the new class names.

use super::super::capture::Snapshot;
use super::super::cascade::{Cascade, SheetInput};
use super::super::css::{self, Sheet};
use super::super::eval::Metrics;
use super::super::verify::{environment, signature};
use super::layers::{ForcedDom, Layer, Overlay, Settle, State, layer_for, layer_of};
use super::par;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, Serialize)]
pub struct StyleDifference {
    pub width: u32,
    pub state: Option<String>,
    pub node: usize,
    pub element: String,
    pub property: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StyleCheck {
    pub widths: Vec<u32>,
    pub states: Vec<String>,
    pub element_states: usize,
    /// Pseudo-element comparisons among `element_states`.
    pub pseudo_elements: usize,
    pub compared: usize,
    pub equal: usize,
    pub different: usize,
    /// Properties skipped because a condition on them cannot be decided from a capture.
    pub skipped_uncertain: usize,
    /// Comparisons not counted because the "before" value was settled to the captured end state or recovered from the captured
    /// computed style (frozen at the resting value): the output carries the same text, so equality
    /// would be a self-comparison, not evidence.
    pub recovered_unverified: usize,
    /// Element-states compared against another captured viewport's own layer.
    pub overlay_compared: usize,
    pub samples: Vec<StyleDifference>,
}

const SAMPLES: usize = 40;

fn describe(snapshot: &Snapshot, node: usize) -> String {
    let Some(e) = snapshot.element(node) else {
        return String::new();
    };
    let class = e
        .attributes
        .iter()
        .find(|(k, _)| k == "class")
        .map_or("", |(_, v)| v.as_str());
    let class: String = class.chars().take(50).collect();
    if class.is_empty() {
        e.tag.clone()
    } else {
        format!("{}.{}", e.tag, class.replace(' ', "."))
    }
}

/// Compare two layers; `revert` on the after side means "no author declaration".
fn compare(
    before: &Layer,
    after: &Layer,
    metrics: &Metrics,
) -> (usize, Vec<(String, String, String)>, usize, usize) {
    let mut equal = 0;
    let mut different = Vec::new();
    let mut skipped = 0;
    let mut recovered = 0;
    let props: BTreeSet<&String> = before.decls.keys().chain(after.decls.keys()).collect();
    for prop in props {
        if before.recovered.iter().any(|r| r == prop) || before.settled.iter().any(|r| r == prop) {
            recovered += 1;
            continue;
        }
        if before.uncertain.contains(prop) || after.uncertain.contains(prop) {
            skipped += 1;
            continue;
        }
        let b = before.decls.get(prop);
        let a = after.decls.get(prop).filter(|v| v.as_str() != "revert");
        match (b, a) {
            (None, None) => {}
            (Some(b), Some(a)) if b == a => equal += 1,
            (Some(b), Some(a)) => {
                let (sb, sa) = (
                    signature(prop, &css::parse_components(b), metrics),
                    signature(prop, &css::parse_components(a), metrics),
                );
                if sb == sa {
                    equal += 1;
                } else {
                    different.push((prop.clone(), b.clone(), a.clone()));
                }
            }
            (Some(b), None) => different.push((prop.clone(), b.clone(), "(none)".into())),
            (None, Some(a)) => different.push((prop.clone(), "(none)".into(), a.clone())),
        }
    }
    (equal, different, skipped, recovered)
}

#[allow(clippy::too_many_arguments)]
pub fn check(
    snapshot: &Snapshot,
    before_inputs: &[SheetInput<'_>],
    after: &Sheet,
    classes: &HashMap<usize, String>,
    nodes: &[usize],
    widths: &[u32],
    states: &BTreeSet<State>,
    state_nodes: &(dyn Fn(usize) -> bool + Sync),
    pseudo_nodes: &[(usize, Vec<String>)],
    settle: &HashMap<usize, Vec<Settle>>,
    overlays: &[Overlay],
) -> StyleCheck {
    let mut result = StyleCheck {
        widths: widths.to_vec(),
        states: states.iter().map(|s| s.variant().to_string()).collect(),
        ..Default::default()
    };
    let metrics = Metrics {
        font_size: 16.0,
        root_font_size: snapshot.root_font_size_px().unwrap_or(16.0),
        vw: f64::from(snapshot.width),
        vh: f64::from(snapshot.height),
        layout_rounding: false,
    };
    let after_inputs = [SheetInput {
        sheet: after,
        index: 0,
        media: "",
    }];
    // Widths that activate the same rules on both sides compare the same thing: check one of each.
    let reduced_motion = states.contains(&State::ReducedMotion);
    let mut seen: BTreeSet<Vec<Vec<u8>>> = BTreeSet::new();
    let unique: Vec<u32> = widths
        .iter()
        .copied()
        .filter(|&w| {
            let mut env = environment(snapshot);
            env.width = f64::from(w);
            let mut fp = vec![
                Cascade::build(before_inputs, &env).fingerprint(),
                Cascade::build(&after_inputs, &env).fingerprint(),
            ];
            // Widths that follow another capture's layer are a different comparison.
            fp.push(
                overlays
                    .iter()
                    .map(|o| u8::from(w >= o.lo && (o.hi == 0 || w < o.hi)))
                    .collect(),
            );
            if reduced_motion {
                env.reduced_motion = true;
                fp.push(Cascade::build(before_inputs, &env).fingerprint());
                fp.push(Cascade::build(&after_inputs, &env).fingerprint());
            }
            seen.insert(fp)
        })
        .collect();
    let parts: Vec<StyleCheck> = par::map_with(
        &unique,
        || (),
        |(), &width| {
            let mut result = StyleCheck::default();
            let mut env = environment(snapshot);
            env.width = f64::from(width);
            let cascade_before = Cascade::build(before_inputs, &env);
            let cascade_after = Cascade::build(&after_inputs, &env);
            env.reduced_motion = true;
            let reduced_before = Cascade::build(before_inputs, &env);
            let reduced_after = Cascade::build(&after_inputs, &env);
            let mut scenarios: Vec<Option<State>> = vec![None];
            scenarios.extend(states.iter().copied().map(Some));
            for state in scenarios {
                for &node in nodes {
                    if state.is_some() && !state_nodes(node) {
                        continue;
                    }
                    let is_motion = state == Some(State::ReducedMotion);
                    let forced = state.filter(|_| !is_motion).map(|s| (node, s));
                    let (cb, ca) = if is_motion {
                        (&reduced_before, &reduced_after)
                    } else {
                        (&cascade_before, &cascade_after)
                    };
                    let dom_before = ForcedDom {
                        inner: snapshot,
                        forced,
                        classes: None,
                    };
                    let dom_after = ForcedDom {
                        inner: snapshot,
                        forced,
                        classes: Some(classes),
                    };
                    let recover = snapshot.computed(node);
                    let mut lb = layer_of(cb, &dom_before, node, recover);
                    if let Some(plan) = settle.get(&node) {
                        lb.apply_settle(plan);
                    }
                    // Widths that follow another captured viewport compare with that capture's own
                    // layer of the paired element.
                    for o in overlays {
                        if width < o.lo || (o.hi != 0 && width >= o.hi) {
                            continue;
                        }
                        let Some(target) = o.layers.get(&node) else {
                            continue;
                        };
                        lb = match state {
                            None => target.clone(),
                            Some(st) => o
                                .states
                                .get(&node)
                                .and_then(|m| m.get(&st))
                                .unwrap_or(target)
                                .clone(),
                        };
                        result.overlay_compared += 1;
                    }
                    let la = layer_of(ca, &dom_after, node, None);
                    result.element_states += 1;
                    let (equal, different, skipped, recovered) = compare(&lb, &la, &metrics);
                    result.recovered_unverified += recovered;
                    result.equal += equal;
                    result.skipped_uncertain += skipped;
                    result.compared += equal + different.len();
                    result.different += different.len();
                    for (property, before, after) in different {
                        if result.samples.len() < SAMPLES {
                            result.samples.push(StyleDifference {
                                width,
                                state: state.map(|s| s.variant().to_string()),
                                node,
                                element: describe(snapshot, node),
                                property,
                                before,
                                after,
                            });
                        }
                    }
                }
            }
            // ::before / ::after at rest.
            for (node, keys) in pseudo_nodes {
                let dom_before = ForcedDom {
                    inner: snapshot,
                    forced: None,
                    classes: None,
                };
                let dom_after = ForcedDom {
                    inner: snapshot,
                    forced: None,
                    classes: Some(classes),
                };
                for key in keys {
                    let recover = snapshot
                        .element(*node)
                        .and_then(|e| e.pseudo.get(key))
                        .map(|s| &snapshot.styles[*s]);
                    let lb = layer_for(&cascade_before, &dom_before, *node, Some(key), recover);
                    let la = layer_for(&cascade_after, &dom_after, *node, Some(key), None);
                    result.element_states += 1;
                    result.pseudo_elements += 1;
                    let (equal, different, skipped, recovered) = compare(&lb, &la, &metrics);
                    result.recovered_unverified += recovered;
                    result.equal += equal;
                    result.skipped_uncertain += skipped;
                    result.compared += equal + different.len();
                    result.different += different.len();
                    for (property, before, after) in different {
                        if result.samples.len() < SAMPLES {
                            result.samples.push(StyleDifference {
                                width,
                                state: Some(format!("::{key}")),
                                node: *node,
                                element: describe(snapshot, *node),
                                property,
                                before,
                                after,
                            });
                        }
                    }
                }
            }
            result
        },
    );
    for part in parts {
        result.element_states += part.element_states;
        result.pseudo_elements += part.pseudo_elements;
        result.compared += part.compared;
        result.equal += part.equal;
        result.different += part.different;
        result.skipped_uncertain += part.skipped_uncertain;
        result.recovered_unverified += part.recovered_unverified;
        result.overlay_compared += part.overlay_compared;
        let room = SAMPLES.saturating_sub(result.samples.len());
        result.samples.extend(part.samples.into_iter().take(room));
    }
    result
}

/// Rule counts that explain what a resting, self-state analysis cannot reach.
pub fn unreachable_state_rules(cascade: &Cascade) -> (usize, Vec<String>) {
    use super::super::selector::Simple;
    fn has_state(simples: &[Simple]) -> bool {
        simples.iter().any(|s| match s {
            Simple::Pseudo(n) => matches!(
                n.as_str(),
                "hover" | "active" | "focus" | "focus-visible" | "focus-within"
            ),
            Simple::Is(l) | Simple::Where(l) | Simple::Not(l) | Simple::Has(l) => {
                l.0.iter()
                    .any(|c| c.compounds.iter().any(|k| has_state(&k.simples)))
            }
            _ => false,
        })
    }
    let mut count = 0;
    let mut samples = BTreeMap::new();
    for rule in &cascade.rules {
        let c = &rule.complex;
        let Some((last, rest)) = c.compounds.split_last() else {
            continue;
        };
        let on_ancestor = rest.iter().any(|k| has_state(&k.simples));
        let on_pseudo_element = last.pseudo_element().is_some() && has_state(&last.simples);
        // `.card:has(a:hover)`: the state sits on a descendant, which is never forced here.
        let on_descendant = last.simples.iter().any(|s| match s {
            Simple::Has(l) => {
                l.0.iter()
                    .any(|c| c.compounds.iter().any(|k| has_state(&k.simples)))
            }
            _ => false,
        });
        if on_ancestor || on_pseudo_element || on_descendant {
            count += 1;
            if samples.len() < 8 {
                samples.insert(c.to_css(), ());
            }
        }
    }
    (count, samples.into_keys().collect())
}
