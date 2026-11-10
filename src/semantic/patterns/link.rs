//! Link captured DOM elements to the JSX call sites that rendered them, using
//! only evidence both sides share: the intrinsic tag, the static class tokens
//! (every token the site always renders must be on the element), static
//! `role` / `type` / `id` / `aria-*` literal props, and literal text props that
//! equal the rendered text. Intrinsic sites (`jsx("button", ..)`) and component
//! call sites (`jsx(Button, ..)`) are linked separately because one element can
//! have both. A tie between different sites is ambiguous and links nothing.
use crate::semantic::ir::{Doc, NodeId};
use crate::semantic::js::{JsFacts, Site};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Link {
    pub file: usize,
    pub site: usize,
    /// Number of agreeing facts (class tokens + static props).
    pub score: usize,
    /// `true` when linked through a linked child element rather than classes.
    pub by_child: bool,
}

#[derive(Debug, Default)]
pub struct Links {
    /// Intrinsic sites (and wrappers inferred from their linked children).
    pub by_node: BTreeMap<NodeId, Link>,
    /// Component call sites (`jsx(Component, props)`).
    pub component: BTreeMap<NodeId, Link>,
    pub ambiguous: usize,
}

const PROP_KEYS: &[&str] = &[
    "role",
    "type",
    "id",
    "aria-label",
    "aria-modal",
    "aria-haspopup",
    "aria-labelledby",
    "data-component",
];

type Cand = (usize, usize, usize);

impl Links {
    pub fn build(doc: &Doc, files: &[JsFacts]) -> Links {
        // class token -> sites that always render it
        let mut by_token: BTreeMap<&str, Vec<(usize, usize)>> = BTreeMap::new();
        for (fi, f) in files.iter().enumerate() {
            for (si, s) in f.sites.iter().enumerate() {
                for t in &s.class_tokens {
                    by_token.entry(t).or_default().push((fi, si));
                }
            }
        }
        let mut links = Links::default();
        for node in &doc.nodes {
            if !node.is_element() || node.classes.is_empty() {
                continue;
            }
            // Candidate sites: any site that always renders one of this element's classes.
            let mut candidates: Vec<(usize, usize)> = node
                .classes
                .iter()
                .filter_map(|c| by_token.get(c.as_str()))
                .filter(|v| v.len() <= 600)
                .flatten()
                .copied()
                .collect();
            candidates.sort_unstable();
            candidates.dedup();
            if candidates.is_empty() {
                continue;
            }
            let mut intrinsic: Vec<Cand> = Vec::new();
            let mut component: Vec<Cand> = Vec::new();
            for &(fi, si) in &candidates {
                let site = &files[fi].sites[si];
                if let Some(score) = score(doc, node.id, site) {
                    if site.intrinsic {
                        intrinsic.push((fi, si, score));
                    } else {
                        component.push((fi, si, score));
                    }
                }
            }
            for (set, is_intrinsic) in [(intrinsic, true), (component, false)] {
                match pick(files, set) {
                    Pick::None => {}
                    Pick::One(l) => {
                        let map = if is_intrinsic {
                            &mut links.by_node
                        } else {
                            &mut links.component
                        };
                        map.insert(node.id, l);
                    }
                    Pick::Ambiguous => links.ambiguous += 1,
                }
            }
        }
        links.link_by_children(doc, files);
        links
    }

    /// Every link of every node: intrinsic first, then component.
    pub fn all(&self) -> impl Iterator<Item = (NodeId, Link)> + '_ {
        self.by_node
            .iter()
            .chain(self.component.iter())
            .map(|(n, l)| (*n, *l))
    }

    pub fn is_linked(&self, node: NodeId) -> bool {
        self.by_node.contains_key(&node) || self.component.contains_key(&node)
    }

    /// Links of one node, intrinsic site first.
    pub fn of(&self, node: NodeId) -> impl Iterator<Item = Link> + '_ {
        self.by_node
            .get(&node)
            .into_iter()
            .chain(self.component.get(&node))
            .copied()
    }

    /// Elements without classes (wrappers) inherit the parent site of their linked
    /// descendants: the nearest ancestor with the parent site's tag, when all
    /// votes agree on one parent site.
    fn link_by_children(&mut self, doc: &Doc, files: &[JsFacts]) {
        for _ in 0..3 {
            let mut votes: BTreeMap<NodeId, BTreeMap<(usize, usize), usize>> = BTreeMap::new();
            for (node, l) in self.all().collect::<Vec<_>>() {
                let site = &files[l.file].sites[l.site];
                let Some(parent) = site.parent else { continue };
                let parent_site = &files[l.file].sites[parent];
                if !parent_site.intrinsic {
                    continue;
                }
                let Some(anc) = doc
                    .ancestors(node)
                    .find(|a| doc.node(*a).tag == parent_site.tag)
                else {
                    continue;
                };
                if self.by_node.contains_key(&anc) {
                    continue;
                }
                *votes
                    .entry(anc)
                    .or_default()
                    .entry((l.file, parent))
                    .or_default() += 1;
            }
            let mut changed = false;
            for (anc, v) in votes {
                if v.len() == 1 {
                    let ((file, site), n) = v.into_iter().next().unwrap();
                    self.by_node.insert(
                        anc,
                        Link {
                            file,
                            site,
                            score: n,
                            by_child: true,
                        },
                    );
                    changed = true;
                } else {
                    self.ambiguous += 1;
                }
            }
            if !changed {
                break;
            }
        }
    }

    pub fn site<'a>(&self, files: &'a [JsFacts], node: NodeId) -> Option<(&'a JsFacts, &'a Site)> {
        let l = self.of(node).next()?;
        Some((&files[l.file], &files[l.file].sites[l.site]))
    }
}

enum Pick {
    None,
    One(Link),
    Ambiguous,
}

fn pick(files: &[JsFacts], cands: Vec<Cand>) -> Pick {
    let Some(best) = cands.iter().map(|c| c.2).max() else {
        return Pick::None;
    };
    let top: Vec<Cand> = cands.into_iter().filter(|c| c.2 == best).collect();
    let first = &files[top[0].0].sites[top[0].1];
    // The same site rendered twice agrees with itself; different sites are ambiguous.
    if top
        .iter()
        .all(|(f, s, _)| same_shape(&files[*f].sites[*s], first))
    {
        Pick::One(Link {
            file: top[0].0,
            site: top[0].1,
            score: best,
            by_child: false,
        })
    } else {
        Pick::Ambiguous
    }
}

fn same_shape(a: &Site, b: &Site) -> bool {
    a.tag == b.tag
        && a.class_tokens == b.class_tokens
        && a.static_props == b.static_props
        && format!("{:?}{:?}", a.handlers, a.bindings)
            == format!("{:?}{:?}", b.handlers, b.bindings)
}

/// `None` when the site contradicts the element; otherwise the count of agreements.
fn score(doc: &Doc, id: NodeId, site: &Site) -> Option<usize> {
    let node = doc.node(id);
    if site.intrinsic && site.tag != node.tag {
        return None;
    }
    if !site.class_tokens.iter().all(|t| node.has_class(t)) {
        return None;
    }
    let mut score = site.class_tokens.len();
    for (k, v) in &site.static_props {
        if !PROP_KEYS.contains(&k.as_str()) {
            continue;
        }
        match node.attr(k) {
            Some(actual) if actual == v => score += 1,
            Some(_) => return None,
            None => {}
        }
    }
    // Literal text props (`text: "Products"`) must agree with the rendered text of a
    // component call: a different literal is a different call.
    let rendered = doc.text_content(id, 200);
    for key in ["text", "label", "title", "children"] {
        if let Some(v) = site.static_props.get(key) {
            if v.is_empty() {
                continue;
            }
            if *v == rendered {
                score += 1;
            } else if !site.intrinsic {
                return None;
            }
        }
    }
    // A component (non-intrinsic) site only counts with a distinctive class.
    (score > 0 && (site.intrinsic || !site.class_tokens.is_empty())).then_some(score)
}
