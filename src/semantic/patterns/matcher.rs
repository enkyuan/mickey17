//! Candidate generation, scoring against the rule table, claiming, and the
//! inventory of controls no rule accepted.
use super::rules::{Candidate, Check, CheckKind, Ctx, RootSel, Rule, eval};
use super::{Evidence, Primitive, Recognition, Unrecognized, bind};
use crate::semantic::ir::{Doc, NodeId, is_interactive_role};
use crate::semantic::js::JsFacts;
use crate::semantic::patterns::{Links, RULES};
use std::collections::{BTreeMap, BTreeSet};

/// Roles owned by a container role (members of the candidate).
fn member_roles(root_role: &str) -> &'static [&'static str] {
    match root_role {
        "tablist" => &["tab"],
        "menu" | "menubar" => &["menuitem", "menuitemcheckbox", "menuitemradio"],
        "radiogroup" => &["radio"],
        _ => &[],
    }
}

/// Descendants with one of `roles` whose nearest ancestor with `owner_role` is `root`.
fn owned(doc: &Doc, root: NodeId, owner_role: &str, roles: &[&str]) -> Vec<NodeId> {
    doc.descendants(root)
        .into_iter()
        .filter(|d| doc.node(*d).role().is_some_and(|r| roles.contains(&r)))
        .filter(|d| {
            doc.ancestors(*d)
                .find(|a| doc.node(*a).role() == Some(owner_role))
                == Some(root)
        })
        .chain(
            doc.node(root)
                .id_refs("aria-owns")
                .into_iter()
                .filter_map(|r| doc.resolve(r))
                .filter(|d| doc.node(*d).role().is_some_and(|r| roles.contains(&r))),
        )
        .collect()
}

/// The panel of a disclosure trigger: `aria-controls` (not an ancestor), a region
/// that names the trigger via `aria-labelledby`, else the adjacent sibling of the
/// trigger (or of its heading wrapper).
fn disclosure_panel(doc: &Doc, trigger: NodeId) -> Option<(NodeId, &'static str)> {
    if let Some(p) = doc
        .controlled(trigger)
        .into_iter()
        .find(|p| !doc.is_ancestor(*p, trigger) && *p != trigger)
    {
        return Some((p, "aria-controls"));
    }
    if let Some(id) = doc.node(trigger).attr("id")
        && let Some(p) = doc.nodes.iter().find(|n| {
            n.is_element() && n.id_refs("aria-labelledby").contains(&id) && n.id != trigger
        })
    {
        return Some((p.id, "labelledby"));
    }
    // Adjacent sibling of the trigger, or of its heading wrapper when the heading
    // holds only the trigger.
    let mut anchor = trigger;
    if let Some(parent) = doc.node(trigger).parent {
        let heading = matches!(doc.node(parent).role(), Some("heading"));
        let only_child = doc.element_children(parent).count() == 1;
        if heading && only_child {
            anchor = parent;
        }
    }
    let parent = doc.node(anchor).parent?;
    let siblings: Vec<NodeId> = doc.element_children(parent).collect();
    let pos = siblings.iter().position(|s| *s == anchor)?;
    let next = *siblings.get(pos + 1)?;
    // The sibling must not itself be a trigger of another disclosure.
    (doc.node(next).aria_bool("aria-expanded").is_none()).then_some((next, "adjacent-sibling"))
}

fn is_popup_trigger(doc: &Doc, n: NodeId) -> bool {
    doc.node(n)
        .attr("aria-haspopup")
        .is_some_and(|v| v != "false")
}

/// Item wrapper of a disclosure: lowest ancestor holding both trigger and panel.
fn item_of(doc: &Doc, trigger: NodeId, panel: NodeId) -> Option<NodeId> {
    doc.lowest_common_ancestor(&[trigger, panel])
}

fn disclosure_candidates(doc: &Doc, claimed: &BTreeSet<NodeId>) -> Vec<Candidate> {
    struct Item {
        trigger: NodeId,
        panel: NodeId,
        via: &'static str,
        item: NodeId,
    }
    let mut items: Vec<Item> = Vec::new();
    for n in &doc.nodes {
        if !n.is_element() || claimed.contains(&n.id) {
            continue;
        }
        if n.aria_bool("aria-expanded").is_none() || is_popup_trigger(doc, n.id) {
            continue;
        }
        if matches!(n.role(), Some("combobox" | "tab" | "treeitem" | "row")) {
            continue;
        }
        let Some((panel, via)) = disclosure_panel(doc, n.id) else {
            continue;
        };
        let Some(item) = item_of(doc, n.id, panel) else {
            continue;
        };
        items.push(Item {
            trigger: n.id,
            panel,
            via,
            item,
        });
    }
    // Items sharing a parent form a group; a lone item is a collapsible.
    let mut groups: BTreeMap<NodeId, Vec<usize>> = BTreeMap::new();
    for (i, it) in items.iter().enumerate() {
        // An item must stay small: a remote panel makes the LCA the page.
        let size = doc.descendants(it.item).len();
        let key = if size > 400 {
            it.trigger
        } else {
            doc.node(it.item).parent.unwrap_or(it.item)
        };
        groups.entry(key).or_default().push(i);
    }
    let mut out = Vec::new();
    for (root, idxs) in groups {
        // Split a group whose members' items overlap.
        let ok = idxs.iter().all(|a| {
            idxs.iter().filter(|b| *a != **b).all(|b| {
                !doc.is_ancestor(items[*a].item, items[*b].item) && items[*a].item != items[*b].item
            })
        });
        if !ok {
            for i in idxs {
                let it = &items[i];
                out.push(Candidate {
                    root: it.item,
                    members: vec![it.trigger],
                    panels: vec![Some(it.panel)],
                    panel_via: vec![it.via],
                    keys: Vec::new(),
                });
            }
            continue;
        }
        // A lone disclosure is a Collapsible whose root is its own item, not the shared parent.
        let root = if idxs.len() == 1 {
            let it = &items[idxs[0]];
            // A remote panel makes the shared ancestor huge: keep the trigger as the root then.
            if doc.descendants(it.item).len() > 400 {
                it.trigger
            } else {
                it.item
            }
        } else {
            root
        };
        out.push(Candidate {
            root,
            members: idxs.iter().map(|i| items[*i].trigger).collect(),
            panels: idxs.iter().map(|i| Some(items[*i].panel)).collect(),
            panel_via: idxs.iter().map(|i| items[*i].via).collect(),
            keys: Vec::new(),
        });
    }
    out
}

fn candidates(rule: &Rule, cx: &Ctx<'_>, claimed: &BTreeSet<NodeId>) -> Vec<Candidate> {
    let doc = cx.doc;
    let free = |n: NodeId| !claimed.contains(&n);
    match rule.root {
        RootSel::Role(roles) => doc
            .nodes
            .iter()
            .filter(|n| {
                n.is_element() && n.role().is_some_and(|r| roles.contains(&r)) && free(n.id)
            })
            .map(|n| Candidate {
                root: n.id,
                members: owned(
                    doc,
                    n.id,
                    n.role().unwrap(),
                    member_roles(n.role().unwrap()),
                )
                .into_iter()
                .filter(|m| free(*m))
                .collect(),
                ..Default::default()
            })
            .collect(),
        RootSel::Disclosure => disclosure_candidates(doc, claimed),
        RootSel::PressedGroup => {
            let mut by_parent: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
            for n in &doc.nodes {
                if n.is_element()
                    && free(n.id)
                    && n.aria_bool("aria-pressed").is_some()
                    && n.role() == Some("button")
                    && let Some(p) = n.parent
                {
                    by_parent.entry(p).or_default().push(n.id);
                }
            }
            by_parent
                .into_iter()
                .map(|(root, members)| Candidate {
                    root,
                    members,
                    ..Default::default()
                })
                .collect()
        }
        RootSel::PopupTrigger => {
            let triggers: Vec<NodeId> = doc
                .nodes
                .iter()
                .filter(|n| n.is_element() && free(n.id) && is_popup_trigger(doc, n.id))
                .map(|n| n.id)
                .collect();
            if rule.primitive == Primitive::NavigationMenu {
                let mut by_nav: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
                for t in triggers {
                    if let Some(nav) = doc
                        .ancestors(t)
                        .find(|a| doc.node(*a).role() == Some("navigation"))
                    {
                        by_nav.entry(nav).or_default().push(t);
                    }
                }
                by_nav
                    .into_iter()
                    .map(|(root, members)| Candidate {
                        root,
                        members,
                        ..Default::default()
                    })
                    .collect()
            } else {
                triggers
                    .into_iter()
                    .map(|t| Candidate {
                        root: t,
                        members: vec![t],
                        ..Default::default()
                    })
                    .collect()
            }
        }
        RootSel::RadioNames => {
            let mut by_name: BTreeMap<String, Vec<NodeId>> = BTreeMap::new();
            for n in &doc.nodes {
                if n.is_element()
                    && free(n.id)
                    && n.tag == "input"
                    && n.attr("type") == Some("radio")
                    && let Some(name) = n.attr("name")
                {
                    by_name.entry(name.to_owned()).or_default().push(n.id);
                }
            }
            by_name
                .into_values()
                .filter_map(|members| {
                    let root = doc.lowest_common_ancestor(&members)?;
                    Some(Candidate {
                        root,
                        members,
                        ..Default::default()
                    })
                })
                .collect()
        }
        RootSel::FormField => doc
            .nodes
            .iter()
            .filter(|n| {
                n.is_element()
                    && free(n.id)
                    && match n.tag.as_str() {
                        "input" => !matches!(
                            n.attr("type").unwrap_or("text"),
                            "hidden"
                                | "checkbox"
                                | "radio"
                                | "button"
                                | "submit"
                                | "reset"
                                | "image"
                        ),
                        "textarea" | "select" => true,
                        _ => false,
                    }
            })
            .map(|n| Candidate {
                root: n.id,
                ..Default::default()
            })
            .collect(),
        RootSel::HoverMenu => {
            use crate::semantic::js::{Ex, Lit, StateEffect};
            let mut groups: BTreeMap<(usize, usize), Vec<(NodeId, String)>> = BTreeMap::new();
            for (node, l) in cx.links.all() {
                if claimed.contains(&node) {
                    continue;
                }
                let site = &cx.js[l.file].sites[l.site];
                for h in site.handlers.iter().filter(|h| h.event == "onMouseEnter") {
                    for e in &h.effects {
                        if let StateEffect::Set {
                            state,
                            value: Ex::Lit { value: Lit::Str(k) },
                        } = e
                        {
                            groups
                                .entry((l.file, *state))
                                .or_default()
                                .push((node, k.clone()));
                        }
                    }
                }
            }
            groups
                .into_values()
                .filter_map(|mut v| {
                    v.sort();
                    v.dedup_by_key(|(n, _)| *n);
                    let keys: std::collections::BTreeSet<&String> =
                        v.iter().map(|(_, k)| k).collect();
                    if keys.len() != v.len() {
                        return None;
                    }
                    let members: Vec<NodeId> = v.iter().map(|(n, _)| *n).collect();
                    let root = doc.lowest_common_ancestor(&members)?;
                    Some(Candidate {
                        root,
                        members,
                        keys: v.into_iter().map(|(_, k)| k).collect(),
                        ..Default::default()
                    })
                })
                .collect()
        }
        RootSel::ButtonLike => doc
            .nodes
            .iter()
            .filter(|n| n.is_element() && free(n.id) && n.role() == Some("button"))
            .map(|n| Candidate {
                root: n.id,
                ..Default::default()
            })
            .collect(),
    }
}

/// Score a candidate: `None` when a required check fails or a contra check vetoes.
fn score(rule: &Rule, cx: &Ctx<'_>, cand: &Candidate) -> (Option<f32>, Vec<Evidence>, Vec<String>) {
    let mut evidence = Vec::new();
    let mut missing = Vec::new();
    let mut sup_total = 0.0f32;
    let mut net = 0.0f32;
    let mut vetoed = false;
    let mut required_ok = true;
    for Check {
        name,
        kind,
        weight,
        pred,
    } in rule.checks
    {
        let hit = eval(*pred, cx, cand);
        match (kind, hit) {
            (CheckKind::Required, Some((source, detail))) => evidence.push(Evidence {
                rule: format!("{}: {name}", rule.id),
                source,
                detail,
                node: Some(cand.root),
                weight: 0.0,
            }),
            (CheckKind::Required, None) => {
                required_ok = false;
                missing.push(format!("required check failed: {name}"));
            }
            (CheckKind::Supporting, hit) => {
                sup_total += weight;
                match hit {
                    Some((source, detail)) => {
                        net += weight;
                        evidence.push(Evidence {
                            rule: format!("{}: {name}", rule.id),
                            source,
                            detail,
                            node: Some(cand.root),
                            weight: *weight,
                        });
                    }
                    None => missing.push(format!("no evidence for: {name}")),
                }
            }
            (CheckKind::Contra, Some((source, detail))) => {
                if *weight >= 1.0 {
                    vetoed = true;
                    missing.push(format!("vetoed by: {name}"));
                }
                net -= weight;
                evidence.push(Evidence {
                    rule: format!("{}: {name}", rule.id),
                    source,
                    detail,
                    node: Some(cand.root),
                    weight: -*weight,
                });
            }
            (CheckKind::Contra, None) => {}
        }
    }
    if !required_ok || vetoed {
        return (None, evidence, missing);
    }
    let conf = if sup_total > 0.0 {
        0.5 + 0.5 * (net / sup_total)
    } else {
        0.5
    };
    (Some(conf.clamp(0.0, 1.0)), evidence, missing)
}

pub fn recognize(doc: &Doc, js: &[JsFacts]) -> Recognition {
    let links = Links::build(doc, js);
    let cx = Ctx {
        doc,
        js,
        links: &links,
    };
    let mut claimed: BTreeSet<NodeId> = BTreeSet::new();
    let mut out = Recognition {
        ambiguous_links: links.ambiguous,
        ..Default::default()
    };
    let mut near: BTreeMap<NodeId, Vec<(String, String)>> = BTreeMap::new();
    for rule in RULES {
        for cand in candidates(rule, &cx, &claimed) {
            let (conf, evidence, missing) = score(rule, &cx, &cand);
            match conf {
                Some(c) if c >= rule.threshold => {
                    let mut m = bind::bind(rule, &cx, &cand, c, evidence);
                    m.id = out.matches.len();
                    for p in &m.parts {
                        claimed.insert(p.node);
                    }
                    claimed.insert(m.root);
                    claimed.extend(cand.members.iter().copied());
                    out.matches.push(m);
                }
                other => {
                    let why = match other {
                        Some(c) => {
                            format!("confidence {c:.2} below threshold {:.2}", rule.threshold)
                        }
                        None => missing.join("; "),
                    };
                    for n in std::iter::once(cand.root).chain(cand.members.iter().copied()) {
                        near.entry(n)
                            .or_default()
                            .push((rule.id.to_owned(), why.clone()));
                    }
                }
            }
        }
    }
    inventory(doc, &claimed, &near, &mut out);
    out
}

/// Count every interactive element and report the unclaimed ones.
fn inventory(
    doc: &Doc,
    claimed: &BTreeSet<NodeId>,
    near: &BTreeMap<NodeId, Vec<(String, String)>>,
    out: &mut Recognition,
) {
    // Nodes that are parts of a recognized match, by primitive.
    let mut owner: BTreeMap<NodeId, Primitive> = BTreeMap::new();
    for m in &out.matches {
        owner.insert(m.root, m.primitive);
        for p in &m.parts {
            owner.insert(p.node, m.primitive);
        }
    }
    for n in &doc.nodes {
        if !n.is_element() {
            continue;
        }
        let role = n.role();
        let interactive = role.is_some_and(is_interactive_role);
        let clickable_plain = !interactive
            && n.style("cursor") == Some("pointer")
            && n.has_attr("tabindex")
            && !doc
                .ancestors(n.id)
                .any(|a| doc.node(a).role().is_some_and(is_interactive_role));
        if !interactive && !clickable_plain {
            continue;
        }
        out.controls += 1;
        if let Some(p) = owner.get(&n.id) {
            *out.controls_by_primitive.entry(*p).or_default() += 1;
            continue;
        }
        if role == Some("link") {
            out.native += 1;
            continue;
        }
        let reason = if clickable_plain {
            "pointer cursor and tabindex without an interactive role: behavior only in script"
                .to_owned()
        } else if claimed.contains(&n.id) {
            "claimed as an inner node of a pattern, but not bound to a part".to_owned()
        } else {
            format!("no rule accepted role `{}`", role.unwrap_or(""))
        };
        out.unrecognized.push(Unrecognized {
            node: n.id,
            tag: n.tag.clone(),
            role: role.map(str::to_owned),
            name: n.name.clone(),
            reason,
            near_misses: near.get(&n.id).cloned().unwrap_or_default(),
        });
    }
}
