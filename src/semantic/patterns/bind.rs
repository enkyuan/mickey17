//! Binders: given an accepted candidate, assign Base UI parts, props and the
//! state model, and record what the original did by hand (primitive-provided)
//! versus what stays application code.
use super::rules::{Candidate, Ctx, Rule, popup_of};
use super::{Evidence, EvidenceSource, JsBinding, Match, Part, Primitive, StateModel};
use crate::semantic::ir::{Doc, NodeId};
use crate::semantic::js::{EffectFact, Ex, JsFacts, KeyRef, Lit, Site, StateEffect, StateId};
use std::collections::BTreeMap;

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

fn part(part: &str, node: NodeId) -> Part {
    Part {
        part: part.to_owned(),
        node,
        key: None,
        key_source: None,
        render_as: None,
    }
}

fn site_of<'a>(cx: &'a Ctx<'_>, node: NodeId) -> Option<(usize, &'a JsFacts, &'a Site)> {
    // Prefer the linked site that carries state or handlers (a component call site often
    // holds the `onClick` while the intrinsic element inside only renders it).
    let links: Vec<_> = cx.links.of(node).collect();
    let l = links
        .iter()
        .find(|l| {
            let s = &cx.js[l.file].sites[l.site];
            !s.bindings.is_empty()
                || !s.handlers.is_empty()
                || !s.conditional_props.is_empty()
                || !s.class_conditional.is_empty()
        })
        .or(links.first())?;
    Some((l.file, &cx.js[l.file], &cx.js[l.file].sites[l.site]))
}

fn ex_states(ex: &Ex, out: &mut Vec<StateId>) {
    match ex {
        Ex::State { state } => out.push(*state),
        Ex::Not { arg } => ex_states(arg, out),
        Ex::Eq { left, right } | Ex::And { left, right } => {
            ex_states(left, out);
            ex_states(right, out)
        }
        Ex::Cond {
            test,
            then,
            otherwise,
        } => {
            ex_states(test, out);
            ex_states(then, out);
            ex_states(otherwise, out)
        }
        _ => {}
    }
}

fn effect_states(e: &StateEffect) -> StateId {
    e.state()
}

/// States a site reads (bindings, conditional props, class modifiers) or moves (handlers).
fn site_states(site: &Site, reads: bool, moves: bool) -> Vec<StateId> {
    let mut out = Vec::new();
    if reads {
        for (_, ex) in &site.bindings {
            ex_states(ex, &mut out);
        }
        for (cond, props) in &site.conditional_props {
            ex_states(cond, &mut out);
            for (_, ex) in props {
                ex_states(ex, &mut out);
            }
        }
        for (_, ex) in &site.class_conditional {
            ex_states(ex, &mut out);
        }
    }
    if moves {
        for h in &site.handlers {
            out.extend(h.effects.iter().map(effect_states));
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn lit_text(l: &Lit) -> String {
    match l {
        Lit::Bool(b) => b.to_string(),
        Lit::Num(n) => n.to_string(),
        Lit::Str(s) => format!("{s:?}"),
        Lit::Null => "null".into(),
        Lit::Undefined => "undefined".into(),
    }
}

/// The initial value of a state exactly as the bundle writes it; `None` when it is a prop or
/// another expression (the capture's value is then only an observation).
fn initial_text(f: &JsFacts, s: StateId) -> Option<String> {
    f.state(s).initial.as_ref().map(lit_text)
}

fn describe_state(f: &JsFacts, s: StateId) -> String {
    let st = f.state(s);
    let init = match (&st.initial, &st.initial_source) {
        (Some(l), _) => lit_text(l),
        (None, Some(src)) => format!("`{src}` (a prop or expression, not a literal)"),
        _ => "unknown".into(),
    };
    format!("state `{}` = useState({init}) at line {}", st.name, st.line)
}

fn describe_effect(f: &JsFacts, e: &StateEffect) -> String {
    let name = |s: StateId| f.state(s).name.clone();
    match e {
        StateEffect::Set { state, value } => {
            format!("sets `{}` to {}", name(*state), ex_text(value))
        }
        StateEffect::Toggle { state } => format!("toggles `{}`", name(*state)),
        StateEffect::SetKey { state, key } => {
            format!("sets `{}` to {}", name(*state), key_text(key))
        }
        StateEffect::ToggleKey { state, key } => {
            format!(
                "toggles `{}` between {} and null",
                name(*state),
                key_text(key)
            )
        }
        StateEffect::Other { state, source } => format!("changes `{}` via {source}", name(*state)),
    }
}

fn key_text(k: &KeyRef) -> String {
    match k {
        KeyRef::LoopItem {
            items: Some(items), ..
        } => format!("the item of the list {items:?}"),
        KeyRef::LoopItem { .. } => "the list item".into(),
        KeyRef::LoopIndex { .. } => "the list index".into(),
        KeyRef::Literal { value } => lit_text(value),
        KeyRef::Unknown { source } => source.clone(),
    }
}

fn ex_text(e: &Ex) -> String {
    match e {
        Ex::Lit { value } => lit_text(value),
        Ex::Key { key } => key_text(key),
        Ex::State { .. } => "a state value".into(),
        Ex::Not { arg } => format!("not {}", ex_text(arg)),
        Ex::Other { source } => source.clone(),
        _ => "an expression".into(),
    }
}

/// What the original's effects do for `state`, split by whether the primitive provides it.
struct Behaviors {
    description: Vec<String>,
    provided: Vec<String>,
    app: Vec<String>,
}

fn behaviors(
    f: &JsFacts,
    state: StateId,
    primitive: Primitive,
    closes_with_false: bool,
) -> Behaviors {
    let st = f.state(state);
    let mut b = Behaviors {
        description: vec![describe_state(f, state)],
        provided: Vec::new(),
        app: Vec::new(),
    };
    for rec in &f.effects {
        if !rec.chain.contains(&st.function) {
            continue;
        }
        match &rec.fact {
            EffectFact::ScrollLock { state: s } if *s == state => {
                if matches!(primitive, Primitive::Dialog | Primitive::AlertDialog) {
                    b.provided.push(
                        "body scroll lock while open (document.body.style.overflow effect)".into(),
                    );
                } else {
                    b.app.push("body scroll lock effect".into());
                }
                b.description.push(format!(
                    "line {}: scroll lock tied to `{}`",
                    rec.line, st.name
                ));
            }
            EffectFact::EscapeKey { effects } if effects.iter().any(|e| e.state() == state) => {
                if matches!(
                    primitive,
                    Primitive::Dialog
                        | Primitive::AlertDialog
                        | Primitive::Popover
                        | Primitive::Menu
                ) {
                    b.provided
                        .push("Escape closes (keydown listener removed)".into());
                }
                b.description.push(format!(
                    "line {}: Escape listener {}",
                    rec.line,
                    effects
                        .iter()
                        .map(|e| describe_effect(f, e))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            EffectFact::MediaQuery { query, effects, .. }
                if effects.iter().any(|e| e.state() == state) =>
            {
                b.app
                    .push(format!("close when media query `{query}` stops matching"));
                b.description.push(format!(
                    "line {}: matchMedia `{query}` change {}",
                    rec.line,
                    effects
                        .iter()
                        .map(|e| describe_effect(f, e))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            EffectFact::DepsChange { deps, effects, .. }
                if effects.iter().any(|e| e.state() == state) =>
            {
                b.app.push(format!(
                    "reset when {} changes (e.g. route change)",
                    deps.join(", ")
                ));
                b.description.push(format!(
                    "line {}: effect on [{}] {}",
                    rec.line,
                    deps.join(", "),
                    effects
                        .iter()
                        .map(|e| describe_effect(f, e))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            _ => {}
        }
    }
    let _ = closes_with_false;
    b
}

fn provided_by(primitive: Primitive) -> Vec<String> {
    let v: &[&str] = match primitive {
        Primitive::Dialog | Primitive::AlertDialog => &[
            "focus trap and restore",
            "outside content inert while modal",
            "role/aria-modal and labelled-by wiring",
        ],
        Primitive::Tabs => &[
            "aria-selected / aria-controls / tabindex wiring",
            "arrow-key and Home/End navigation",
        ],
        Primitive::Accordion | Primitive::Collapsible => &[
            "aria-expanded / aria-controls wiring",
            "Enter/Space activation, arrow-key navigation (Accordion)",
        ],
        Primitive::ToggleGroup | Primitive::Toggle => &["aria-pressed and roving focus"],
        Primitive::Switch | Primitive::Checkbox | Primitive::RadioGroup => {
            &["role/aria-checked wiring, keyboard activation, form participation"]
        }
        Primitive::NavigationMenu | Primitive::Menu | Primitive::Menubar => &[
            "open/close, hover intent, focus management",
            "arrow-key navigation",
        ],
        _ => &[],
    };
    v.iter().map(|s| (*s).to_owned()).collect()
}

/// A short label for key derivation: the accessible name when the node renders one
/// text run, else its first text run (a card-like button's title), cut at six words.
fn short_label(doc: &Doc, node: NodeId) -> Option<String> {
    let name = doc.node(node).name.clone();
    let runs: Vec<&str> = doc
        .descendants(node)
        .into_iter()
        .filter(|d| {
            !doc.ancestors(*d)
                .take_while(|a| *a != node)
                .any(|a| doc.node(a).aria_bool("aria-hidden") == Some(true))
        })
        .filter_map(|d| {
            doc.node(d)
                .text
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
        })
        .collect();
    match (runs.len(), name) {
        (n, Some(name)) if n <= 1 && name.split_whitespace().count() <= 6 => Some(name),
        (_, name) => runs
            .first()
            .map(|t| t.split_whitespace().take(6).collect::<Vec<_>>().join(" "))
            .or(name),
    }
}

fn unique_keys(names: Vec<String>) -> Vec<String> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    names
        .into_iter()
        .map(|n| {
            let c = seen.entry(n.clone()).or_default();
            *c += 1;
            if *c == 1 { n } else { format!("{n}-{c}") }
        })
        .collect()
}

/// Keys for `members`: the static list the bundle maps over when the members all come
/// from one loop site of matching length, else a slug of each accessible name.
fn member_keys(cx: &Ctx<'_>, members: &[NodeId]) -> (Vec<String>, String) {
    let doc = cx.doc;
    let linked: Vec<_> = members.iter().map(|m| cx.links.of(*m).next()).collect();
    if let Some(Some(first)) = linked.first()
        && linked
            .iter()
            .all(|l| l.map(|l| (l.file, l.site)) == Some((first.file, first.site)))
    {
        let site = &cx.js[first.file].sites[first.site];
        for &lp in site.loops.iter().rev() {
            let info = &cx.js[first.file].loops[lp];
            if let Some(items) = &info.items
                && items.len() == members.len()
            {
                return (
                    items.clone(),
                    format!(
                        "static list mapped by the bundle (`{}`, line {})",
                        info.source, info.line
                    ),
                );
            }
        }
    }
    let names: Vec<String> = members
        .iter()
        .map(|m| {
            short_label(doc, *m)
                .as_deref()
                .map(slug)
                .filter(|s| !s.is_empty())
                .unwrap_or_default()
        })
        .collect();
    if names.iter().all(|n| !n.is_empty()) {
        (unique_keys(names), "slug of the accessible name".into())
    } else {
        (
            (0..members.len())
                .map(|i| format!("item-{}", i + 1))
                .collect(),
            "position (no accessible name available)".into(),
        )
    }
}

fn bound(
    rule: &Rule,
    cand: &Candidate,
    conf: f32,
    evidence: Vec<Evidence>,
    label: Option<String>,
) -> Match {
    Match {
        id: 0,
        primitive: rule.primitive,
        rule: rule.id.to_owned(),
        root: cand.root,
        label,
        parts: Vec::new(),
        props: BTreeMap::new(),
        state: StateModel::Static,
        confidence: conf,
        evidence,
        gaps: Vec::new(),
        js: None,
    }
}

fn dom_evidence(m: &mut Match, rule: &str, source: EvidenceSource, detail: String, node: NodeId) {
    m.evidence.push(Evidence {
        rule: rule.to_owned(),
        source,
        detail,
        node: Some(node),
        weight: 0.0,
    });
}

pub fn bind(
    rule: &Rule,
    cx: &Ctx<'_>,
    cand: &Candidate,
    conf: f32,
    evidence: Vec<Evidence>,
) -> Match {
    let label = cx.doc.node(cand.root).name.clone();
    let mut m = bound(rule, cand, conf, evidence, label);
    match rule.primitive {
        Primitive::Tabs => tabs(cx, cand, &mut m),
        Primitive::ToggleGroup | Primitive::Toggle => toggle(cx, cand, &mut m),
        Primitive::Accordion | Primitive::Collapsible => disclosure(cx, cand, &mut m),
        Primitive::Dialog | Primitive::AlertDialog => dialog(cx, cand, &mut m),
        Primitive::Switch | Primitive::Checkbox => checkable(cx, cand, &mut m),
        Primitive::RadioGroup => radio_group(cx, cand, &mut m),
        Primitive::Combobox => combobox(cx, cand, &mut m),
        Primitive::Select => select(cx, cand, &mut m),
        Primitive::NavigationMenu => navigation(cx, cand, &mut m),
        Primitive::Menu | Primitive::Menubar => menu(cx, cand, &mut m),
        Primitive::Popover => popover(cx, cand, &mut m),
        Primitive::Tooltip => tooltip(cx, cand, &mut m),
        Primitive::Field => {
            m.parts.push(part("Root", cand.root));
            m.parts.push(part("Control", cand.root));
            if cx.doc.node(cand.root).name.is_none() {
                m.gaps.push(
                    "form control has no accessible name; Field.Label must be authored".into(),
                );
            }
        }
        Primitive::Button => {
            m.parts.push(part("", cand.root));
            if cx.doc.node(cand.root).tag != "button" {
                m.parts[0].render_as = Some(cx.doc.node(cand.root).tag.clone());
                m.gaps.push(format!(
                    "role=button on <{}>: Base UI Button needs `nativeButton={{false}}` with this element",
                    cx.doc.node(cand.root).tag
                ));
            }
        }
    }
    m
}

fn tabs(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    let doc = cx.doc;
    let (keys, source) = member_keys(cx, &cand.members);
    if source.starts_with("position") {
        m.gaps.push("item keys are positional (`item-N`): the items have no accessible name or static key list to derive a semantic key from".into());
    }
    let mut panels: Vec<(usize, NodeId)> = Vec::new();
    for (i, t) in cand.members.iter().enumerate() {
        let panel = doc
            .controlled(*t)
            .into_iter()
            .find(|p| doc.node(*p).role() == Some("tabpanel"))
            .or_else(|| {
                doc.node(*t).attr("id").and_then(|id| {
                    doc.nodes
                        .iter()
                        .find(|n| {
                            n.role() == Some("tabpanel")
                                && n.id_refs("aria-labelledby").contains(&id)
                        })
                        .map(|n| n.id)
                })
            });
        if let Some(p) = panel {
            panels.push((i, p));
        }
    }
    let mut root_nodes = vec![cand.root];
    root_nodes.extend(panels.iter().map(|(_, p)| *p));
    let root = doc.lowest_common_ancestor(&root_nodes).unwrap_or(cand.root);
    m.parts.push(part("Root", root));
    m.parts.push(part("List", cand.root));
    for (i, t) in cand.members.iter().enumerate() {
        let mut p = part("Tab", *t);
        p.key = Some(keys[i].clone());
        p.key_source = Some(source.clone());
        if doc.node(*t).tag != "button" {
            p.render_as = Some(doc.node(*t).tag.clone());
        }
        m.parts.push(p);
    }
    for (i, p) in &panels {
        let mut pp = part("Panel", *p);
        pp.key = Some(keys[*i].clone());
        pp.key_source = Some(source.clone());
        m.parts.push(pp);
    }
    if panels.len() != cand.members.len() {
        m.gaps.push(format!(
            "{} of {} tabs have no tabpanel in the DOM/ARIA; panel content per value is not linked (the original swaps content by script). Tabs.Root wraps the list only",
            cand.members.len() - panels.len(),
            cand.members.len()
        ));
    }
    if let Some(o) = doc.node(cand.root).attr("aria-orientation") {
        m.props.insert("orientation".into(), o.to_owned());
    }
    let selected = cand
        .members
        .iter()
        .position(|t| doc.node(*t).aria_bool("aria-selected") == Some(true));
    let default = selected.map(|i| keys[i].clone());
    // State from the bundle: a state variable the tab sites bind or move.
    let mut state_model = StateModel::Uncontrolled {
        default: default.clone(),
        observed_from: "the tab with aria-selected=true in the capture".into(),
    };
    if let Some((fi, f, s)) = cand
        .members
        .iter()
        .find_map(|t| site_of(cx, *t))
        .map(|(fi, f, site)| (fi, f, site_states(site, true, true)))
        .and_then(|(fi, f, s)| s.first().copied().map(|s| (fi, f, s)))
    {
        let b = behaviors(f, s, Primitive::Tabs, false);
        state_model = StateModel::Controlled {
            state: f.state(s).name.clone(),
            initial: initial_text(f, s),
            observed: default.clone(),
            moved_by_others: b.app.clone(),
        };
        m.js = Some(JsBinding {
            file: f.file.clone(),
            state: Some(s),
            description: b.description,
            provided_by_primitive: provided_by(Primitive::Tabs),
            app_code: b.app,
        });
        let _ = fi;
    } else {
        m.js = None;
    }
    m.state = state_model;
    if let Some(d) = default {
        m.props.insert("defaultValue".into(), d);
    }
}

fn toggle(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    let doc = cx.doc;
    if m.primitive == Primitive::Toggle {
        m.parts.push(part("", cand.members[0]));
        let pressed = doc.node(cand.members[0]).aria_bool("aria-pressed") == Some(true);
        m.props.insert("defaultPressed".into(), pressed.to_string());
        m.state = StateModel::Uncontrolled {
            default: Some(pressed.to_string()),
            observed_from: "aria-pressed in the capture".into(),
        };
        return;
    }
    let (keys, source) = member_keys(cx, &cand.members);
    m.parts.push(part("", cand.root));
    for (i, t) in cand.members.iter().enumerate() {
        let mut p = part("Toggle", *t);
        p.key = Some(keys[i].clone());
        p.key_source = Some(source.clone());
        m.parts.push(p);
    }
    let pressed: Vec<usize> = cand
        .members
        .iter()
        .enumerate()
        .filter(|(_, t)| doc.node(**t).aria_bool("aria-pressed") == Some(true))
        .map(|(i, _)| i)
        .collect();
    m.props
        .insert("multiple".into(), (pressed.len() > 1).to_string());
    if !pressed.is_empty() {
        m.props.insert(
            "defaultValue".into(),
            format!(
                "[{}]",
                pressed
                    .iter()
                    .map(|i| format!("{:?}", keys[*i]))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
    m.gaps.push(
        "ToggleGroup keeps the original aria-pressed semantics; when the buttons switch which content is visible, Tabs would be the more accurate role, but the DOM does not say so".into(),
    );
    m.state = StateModel::Uncontrolled {
        default: pressed.first().map(|i| keys[*i].clone()),
        observed_from: "aria-pressed in the capture".into(),
    };
    if let Some((f, state)) = cand.members.iter().find_map(|t| {
        let (_, f, site) = site_of(cx, *t)?;
        Some((f, *site_states(site, true, true).first()?))
    }) {
        let b = behaviors(f, state, Primitive::ToggleGroup, false);
        m.state = StateModel::Controlled {
            state: f.state(state).name.clone(),
            initial: initial_text(f, state),
            observed: pressed.first().map(|i| keys[*i].clone()),
            moved_by_others: b.app.clone(),
        };
        m.js = Some(JsBinding {
            file: f.file.clone(),
            state: Some(state),
            description: b.description,
            provided_by_primitive: provided_by(Primitive::ToggleGroup),
            app_code: b.app,
        });
    }
}

fn heading_wrapper(doc: &Doc, trigger: NodeId, item: NodeId) -> Option<NodeId> {
    let parent = doc.node(trigger).parent?;
    (parent != item
        && doc.node(parent).role() == Some("heading")
        && doc.element_children(parent).count() == 1)
        .then_some(parent)
}

fn disclosure(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    let doc = cx.doc;
    let accordion = m.primitive == Primitive::Accordion;
    let (keys, source) = member_keys(cx, &cand.members);
    m.parts.push(part("Root", cand.root));
    let mut any_collapsed_panel_present = false;
    for (i, trigger) in cand.members.iter().enumerate() {
        let panel = cand.panels[i].expect("disclosure candidates carry panels");
        let item = doc
            .lowest_common_ancestor(&[*trigger, panel])
            .unwrap_or(cand.root);
        if accordion {
            let mut it = part("Item", item);
            it.key = Some(keys[i].clone());
            it.key_source = Some(source.clone());
            if doc.node(item).tag != "div" {
                it.render_as = Some(doc.node(item).tag.clone());
            }
            m.parts.push(it);
            if let Some(h) = heading_wrapper(doc, *trigger, item) {
                let mut hp = part("Header", h);
                hp.render_as = Some(doc.node(h).tag.clone());
                m.parts.push(hp);
            }
        }
        m.parts.push(part("Trigger", *trigger));
        m.parts.push(part("Panel", panel));
        if doc.node(*trigger).aria_bool("aria-expanded") == Some(false) {
            any_collapsed_panel_present = true;
        }
    }
    if any_collapsed_panel_present {
        m.props.insert("keepMounted".into(), "true".into());
        dom_evidence(
            m,
            "disclosure.keep-mounted",
            EvidenceSource::Dom,
            "collapsed panels are present in the captured DOM".into(),
            cand.root,
        );
    }
    let expanded: Vec<usize> = cand
        .members
        .iter()
        .enumerate()
        .filter(|(_, t)| doc.node(**t).aria_bool("aria-expanded") == Some(true))
        .map(|(i, _)| i)
        .collect();
    // Single vs multiple, and whether the open item can be closed again, from the bundle.
    let mut closable: Option<bool> = None;
    let mut single: Option<bool> = None;
    let mut js_state: Option<(&JsFacts, StateId)> = None;
    for t in &cand.members {
        let Some((_, f, site)) = site_of(cx, *t) else {
            continue;
        };
        for h in &site.handlers {
            for e in &h.effects {
                match e {
                    StateEffect::ToggleKey { state, .. } => {
                        single = Some(true);
                        closable = Some(true);
                        js_state = Some((f, *state));
                    }
                    StateEffect::SetKey { state, .. } => {
                        single = Some(true);
                        closable = Some(false);
                        js_state = Some((f, *state));
                    }
                    StateEffect::Toggle { state } => {
                        // One boolean per item: independent items.
                        single = Some(false);
                        closable = Some(true);
                        js_state = Some((f, *state));
                    }
                    _ => {}
                }
            }
        }
        if js_state.is_some() {
            break;
        }
    }
    if accordion {
        let multiple = match single {
            Some(s) => !s,
            None if expanded.len() > 1 => true,
            None => {
                m.gaps.push("single vs multiple open items is not observable without the original script; defaulting to single (Base UI default)".into());
                false
            }
        };
        m.props.insert("multiple".into(), multiple.to_string());
        if closable == Some(false) {
            m.gaps.push("the original never closes the open item by activating it again; Base UI Accordion allows it, so onValueChange must ignore an empty value (app code)".into());
        }
        if !expanded.is_empty() {
            m.props.insert(
                "defaultValue".into(),
                format!(
                    "[{}]",
                    expanded
                        .iter()
                        .map(|i| format!("{:?}", keys[*i]))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
    } else {
        m.props
            .insert("defaultOpen".into(), (!expanded.is_empty()).to_string());
    }
    m.state = StateModel::Uncontrolled {
        default: expanded.first().map(|i| keys[*i].clone()),
        observed_from: "aria-expanded in the capture".into(),
    };
    if let Some((f, state)) = js_state {
        let b = behaviors(f, state, m.primitive, false);
        let mut app = b.app;
        let others: Vec<String> = other_movers(cx, f, state, &cand.members)
            .into_iter()
            .map(|(d, _)| d)
            .collect();
        app.extend(others.clone());
        m.state = StateModel::Controlled {
            state: f.state(state).name.clone(),
            initial: initial_text(f, state),
            observed: expanded.first().map(|i| keys[*i].clone()),
            moved_by_others: others,
        };
        m.js = Some(JsBinding {
            file: f.file.clone(),
            state: Some(state),
            description: b.description,
            provided_by_primitive: provided_by(m.primitive),
            app_code: app,
        });
    }
}

/// Handlers outside `members` that also write `state` (a dialog close that resets an accordion).
/// Handlers outside `members` that write `state`: description and whether it closes (sets false).
fn other_movers(
    cx: &Ctx<'_>,
    f: &JsFacts,
    state: StateId,
    members: &[NodeId],
) -> Vec<(String, bool)> {
    let own: Vec<NodeId> = members.to_vec();
    let mut out = Vec::new();
    for (node, l) in cx.links.all() {
        if own.contains(&node) || cx.js[l.file].file != f.file {
            continue;
        }
        let site = &cx.js[l.file].sites[l.site];
        for h in &site.handlers {
            for e in &h.effects {
                if e.state() == state {
                    let closes = matches!(
                        e,
                        StateEffect::Set {
                            value: Ex::Lit {
                                value: Lit::Bool(false)
                            },
                            ..
                        }
                    );
                    out.push((
                        format!(
                            "`{}` {} on <{}> {}",
                            h.event,
                            describe_effect(f, e),
                            site.tag,
                            site.class_tokens
                                .first()
                                .map(|c| format!(".{c}"))
                                .unwrap_or_default()
                        ),
                        closes,
                    ));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn dialog(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    let doc = cx.doc;
    let root = doc.node(cand.root);
    m.parts.push(part("Root", cand.root));
    m.parts.push(part("Popup", cand.root));
    m.props.insert(
        "modal".into(),
        (root.aria_bool("aria-modal") == Some(true)).to_string(),
    );
    // Triggers by ARIA first.
    let mut triggers: Vec<NodeId> = doc.controllers_of(cand.root);
    triggers.retain(|t| !doc.is_ancestor(cand.root, *t));
    // The open state of the original: the state its inert/open modifier reads.
    let mut state: Option<(&JsFacts, StateId)> = None;
    if let Some((_, f, site)) = site_of(cx, cand.root)
        && let Some(s) = site_states(site, true, false).first()
    {
        state = Some((f, *s));
    }
    if let Some((f, s)) = state {
        for (node, l) in cx.links.all() {
            if cx.js[l.file].file != f.file || doc.is_ancestor(cand.root, node) || node == cand.root
            {
                continue;
            }
            let site = &cx.js[l.file].sites[l.site];
            let opens = site
                .handlers
                .iter()
                .flat_map(|h| &h.effects)
                .any(|e| match e {
                    StateEffect::Toggle { state } => *state == s,
                    StateEffect::Set {
                        state,
                        value:
                            Ex::Lit {
                                value: Lit::Bool(true),
                            },
                    } => *state == s,
                    _ => false,
                });
            if opens && !triggers.contains(&node) {
                triggers.push(node);
            }
        }
    }
    triggers.sort_unstable();
    triggers.dedup();
    for t in &triggers {
        m.parts.push(part("Trigger", *t));
    }
    if triggers.is_empty() {
        m.gaps.push("no trigger found: neither aria-controls/aria-haspopup nor a recovered handler opens this dialog".into());
    }
    if doc
        .descendants(cand.root)
        .into_iter()
        .all(|d| doc.node(d).tag != "h2" && doc.node(d).role() != Some("heading"))
    {
        m.gaps.push("no heading inside the dialog: Dialog.Title is not bound; the accessible name comes from aria-label".into());
    }
    m.gaps
        .push("no backdrop element in the captured DOM; Dialog.Backdrop is not emitted".into());
    m.state = StateModel::Uncontrolled {
        default: Some("closed".into()),
        observed_from: "inert/hidden in the capture".into(),
    };
    if let Some((f, s)) = state {
        let b = behaviors(f, s, m.primitive, true);
        let mut app = b.app.clone();
        let others = other_movers(cx, f, s, &triggers);
        // Closers inside the dialog (links that close on activate) stay application code.
        let closers: Vec<String> = others
            .into_iter()
            .filter(|(_, closes)| *closes)
            .map(|(d, _)| d)
            .collect();
        app.extend(closers.iter().cloned());
        m.state = StateModel::Controlled {
            state: f.state(s).name.clone(),
            initial: initial_text(f, s),
            observed: Some("closed".into()),
            moved_by_others: app.clone(),
        };
        m.js = Some(JsBinding {
            file: f.file.clone(),
            state: Some(s),
            description: b.description,
            provided_by_primitive: {
                let mut p = b.provided.clone();
                p.extend(provided_by(m.primitive));
                p
            },
            app_code: app,
        });
    }
}

fn checkable(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    let doc = cx.doc;
    let n = doc.node(cand.root);
    m.parts.push(part("Root", cand.root));
    if m.primitive == Primitive::Switch {
        m.parts.push(part("Thumb", cand.root));
        if let Some(p) = m.parts.last_mut() {
            p.render_as = Some("(not in DOM)".into());
        }
        m.gaps.push("Switch.Thumb is a styling element absent from the captured DOM; emitted as part of the composition".into());
    }
    let checked = n.aria_bool("aria-checked").or(Some(n.has_attr("checked")));
    if let Some(c) = checked {
        m.props.insert("defaultChecked".into(), c.to_string());
        m.state = StateModel::Uncontrolled {
            default: Some(c.to_string()),
            observed_from: "aria-checked/checked in the capture".into(),
        };
    }
    if n.tag == "input" {
        m.gaps.push("native <input> is replaced by the primitive's own hidden input; name/value/form attributes carry over".into());
    }
    if n.name.is_none() {
        m.gaps
            .push("no accessible name: a label must be authored".into());
    }
}

fn radio_group(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    let doc = cx.doc;
    m.parts.push(part("", cand.root));
    for r in &cand.members {
        m.parts.push(part("Radio", *r));
    }
    let checked = cand.members.iter().position(|r| {
        doc.node(*r).aria_bool("aria-checked") == Some(true) || doc.node(*r).has_attr("checked")
    });
    if let Some(i) = checked {
        let v = doc
            .node(cand.members[i])
            .attr("value")
            .map(str::to_owned)
            .unwrap_or_else(|| format!("item-{}", i + 1));
        m.props.insert("defaultValue".into(), v);
    }
    m.state = StateModel::Uncontrolled {
        default: None,
        observed_from: "checked radio in the capture".into(),
    };
}

fn combobox(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    let doc = cx.doc;
    m.parts.push(part("Root", cand.root));
    m.parts.push(part("Input", cand.root));
    if let Some(list) = doc
        .controlled(cand.root)
        .into_iter()
        .find(|n| doc.node(*n).role() == Some("listbox"))
    {
        m.parts.push(part("List", list));
        for opt in doc
            .descendants(list)
            .into_iter()
            .filter(|d| doc.node(*d).role() == Some("option"))
        {
            m.parts.push(part("Item", opt));
        }
        if doc.node(list).tag.contains('-') {
            m.gaps.push(format!("listbox is a custom element <{}>: its options are rendered by script and are absent from the capture", doc.node(list).tag));
        }
    } else {
        m.gaps
            .push("combobox popup list not found in the DOM".into());
    }
    m.gaps.push(
        "option source (static list vs async suggestions) is not observable in the capture".into(),
    );
    m.state = StateModel::Uncontrolled {
        default: None,
        observed_from: "empty input in the capture".into(),
    };
}

fn select(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    let doc = cx.doc;
    m.parts.push(part("Root", cand.root));
    m.parts.push(part("Trigger", cand.root));
    if doc.node(cand.root).tag == "select" {
        for opt in doc
            .descendants(cand.root)
            .into_iter()
            .filter(|d| doc.node(*d).tag == "option")
        {
            m.parts.push(part("Item", opt));
        }
    } else if let Some(list) = doc.controlled(cand.root).into_iter().next() {
        m.parts.push(part("List", list));
    } else {
        m.gaps.push("select popup list not found in the DOM".into());
    }
    m.state = StateModel::Uncontrolled {
        default: None,
        observed_from: "selected option in the capture".into(),
    };
}

fn hover_navigation(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    let doc = cx.doc;
    m.parts.push(part("Root", cand.root));
    let parents: Vec<Option<NodeId>> = cand.members.iter().map(|n| doc.node(*n).parent).collect();
    if let Some(Some(list)) = parents.first().copied()
        && parents.iter().all(|p| *p == Some(list))
    {
        m.parts.push(part("List", list));
    }
    for (i, item) in cand.members.iter().enumerate() {
        let mut ip = part("Item", *item);
        ip.key = Some(cand.keys[i].clone());
        ip.key_source = Some("string set by the pointer-enter handler in the bundle".into());
        m.parts.push(ip);
        let trigger = std::iter::once(*item)
            .chain(doc.descendants(*item))
            .find(|d| matches!(doc.node(*d).role(), Some("button" | "link")));
        match trigger {
            Some(t) => {
                let mut tp = part("Trigger", t);
                tp.key = Some(cand.keys[i].clone());
                m.parts.push(tp);
            }
            None => m.gaps.push(format!(
                "item {} has no button or link to act as Trigger",
                cand.keys[i]
            )),
        }
    }
    m.gaps.push("dropdown content is rendered by script while hovered and is absent from the captured DOM; NavigationMenu.Content bodies cannot be bound".into());
    m.gaps.push("pointer-intent timing (enter/leave delays) of the original is not recovered; Base UI uses its own delay props".into());
    m.state = StateModel::Uncontrolled {
        default: Some("closed".into()),
        observed_from: "no dropdown in the capture".into(),
    };
    if let Some((f, state)) = cand.members.first().and_then(|n| {
        let (_, f, site) = site_of(cx, *n)?;
        let s = site
            .handlers
            .iter()
            .flat_map(|h| &h.effects)
            .map(|e| e.state())
            .next()?;
        Some((f, s))
    }) {
        let mut description = vec![describe_state(f, state)];
        let mut app = Vec::new();
        for site in &f.sites {
            if site_states(site, true, false).contains(&state) {
                description.push(format!(
                    "line {}: <{}> reads the open key and renders the dropdown",
                    site.line, site.tag
                ));
                app.push(format!(
                    "shared dropdown rendered from `{}` (component `{}`)",
                    f.state(state).name,
                    site.tag
                ));
            }
            for h in &site.handlers {
                if h.event == "onClose" {
                    description.push(format!(
                        "<{}> onClose: {}",
                        site.tag,
                        h.effects
                            .iter()
                            .map(|e| describe_effect(f, e))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }
        m.state = StateModel::Controlled {
            state: f.state(state).name.clone(),
            initial: initial_text(f, state),
            observed: None,
            moved_by_others: app.clone(),
        };
        m.js = Some(JsBinding {
            file: f.file.clone(),
            state: Some(state),
            description,
            provided_by_primitive: provided_by(Primitive::NavigationMenu),
            app_code: app,
        });
    }
}

fn navigation(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    if !cand.keys.is_empty() {
        return hover_navigation(cx, cand, m);
    }
    let doc = cx.doc;
    m.parts.push(part("Root", cand.root));
    if let Some(list) = cand
        .members
        .iter()
        .filter_map(|t| doc.node(*t).parent)
        .next()
        .and_then(|p| {
            doc.ancestors(p)
                .find(|a| doc.node(*a).role() == Some("list"))
                .or(Some(p))
        })
    {
        m.parts.push(part("List", list));
    }
    for t in &cand.members {
        let item = doc.node(*t).parent.unwrap_or(*t);
        m.parts.push(part("Item", item));
        m.parts.push(part("Trigger", *t));
        match popup_of(doc, *t) {
            Some(p) => m.parts.push(part("Content", p)),
            None => m.gaps.push(format!(
                "trigger node {t} has no content element (no aria-controls, no sibling)"
            )),
        }
    }
    m.gaps.push("trigger-to-content association is structural (adjacent sibling) where aria-controls is absent".into());
    m.state = StateModel::Uncontrolled {
        default: Some("closed".into()),
        observed_from: "aria-expanded=false in the capture".into(),
    };
}

fn menu(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    m.parts.push(part("Root", cand.root));
    m.parts.push(part("Popup", cand.root));
    for item in &cand.members {
        m.parts.push(part("Item", *item));
    }
    // Triggers: elements that name the menu through aria-controls/aria-owns.
    let triggers = cx.doc.controllers_of(cand.root);
    for t in &triggers {
        m.parts.push(part("Trigger", *t));
    }
    if triggers.is_empty() {
        m.gaps
            .push("no trigger references the menu through aria-controls".into());
    }
    m.state = StateModel::Uncontrolled {
        default: Some("closed".into()),
        observed_from: "capture".into(),
    };
}

fn popover(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    let doc = cx.doc;
    m.parts.push(part("Root", cand.root));
    m.parts.push(part("Trigger", cand.root));
    match popup_of(doc, cand.root) {
        Some(p) => m.parts.push(part("Popup", p)),
        None => m.gaps.push("popup element not found".into()),
    }
    m.state = StateModel::Uncontrolled {
        default: Some("closed".into()),
        observed_from: "aria-expanded in the capture".into(),
    };
}

fn tooltip(cx: &Ctx<'_>, cand: &Candidate, m: &mut Match) {
    let doc = cx.doc;
    m.parts.push(part("Root", cand.root));
    m.parts.push(part("Popup", cand.root));
    let id = doc.node(cand.root).attr("id").unwrap_or_default();
    match doc
        .nodes
        .iter()
        .find(|n| n.is_element() && n.id_refs("aria-describedby").contains(&id) && !id.is_empty())
    {
        Some(t) => m.parts.push(part("Trigger", t.id)),
        None => m
            .gaps
            .push("no element references the tooltip through aria-describedby".into()),
    }
}
