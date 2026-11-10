//! Repeated-component detection. Subtrees are hashed structurally (a Merkle hash
//! over tag, role, class blocks, attribute names and child hashes, ignoring text,
//! attribute values and state modifiers). Equal hashes are repeated instances;
//! positions where their text or attributes differ become typed props. Near
//! duplicates (different hashes, small tree-edit distance) are reported as
//! variants. A name that cannot be derived from `data-component`, BEM classes,
//! roles, headings or structure stays unresolved: the group is reported, not emitted.
mod naming;
mod ted;

use super::ir::{Doc, NodeId};
use serde::Serialize;
use std::collections::BTreeMap;

pub use naming::{camel, element_at, identifier_or_pascal, pascal, words};
pub use ted::{TreeEdit, tree_edit_distance};

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn mix(h: u64, bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(h, |h, b| (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME))
}

/// Attributes that carry state or identity, not structure.
const STATE_ATTRS: &[&str] = &[
    "id",
    "class",
    "style",
    "aria-expanded",
    "aria-selected",
    "aria-pressed",
    "aria-checked",
    "aria-hidden",
    "aria-current",
    "aria-controls",
    "aria-labelledby",
    "tabindex",
    "inert",
    "hidden",
    "data-state",
    "key",
];

const UTILITY_PREFIXES: &[&str] = &[
    "space-",
    "w-",
    "h-",
    "m-",
    "mx-",
    "my-",
    "mt-",
    "mb-",
    "ml-",
    "mr-",
    "p-",
    "px-",
    "py-",
    "pt-",
    "pb-",
    "pl-",
    "pr-",
    "flex",
    "grid",
    "inline",
    "block",
    "hidden",
    "text-",
    "bg-",
    "border",
    "rounded",
    "shadow",
    "font-",
    "leading-",
    "tracking-",
    "gap-",
    "col-",
    "row-",
    "items-",
    "justify-",
    "self-",
    "order-",
    "overflow",
    "absolute",
    "relative",
    "fixed",
    "sticky",
    "top-",
    "left-",
    "right-",
    "bottom-",
    "inset-",
    "z-",
    "opacity-",
    "min-",
    "max-",
    "size-",
    "object-",
    "cursor-",
    "pointer-events",
    "select-",
    "transition",
    "duration-",
    "ease-",
    "delay-",
    "transform",
    "translate-",
    "scale-",
    "rotate-",
    "outline",
    "ring",
    "focus",
    "hover",
    "group",
    "peer",
    "sr-only",
    "truncate",
    "underline",
    "uppercase",
    "lowercase",
    "capitalize",
    "whitespace-",
    "break-",
    "list-",
    "table",
    "float-",
    "clear-",
    "fill-",
    "stroke-",
    "aspect-",
    "container",
    "visible",
    "invisible",
    "static",
    "content-",
    "place-",
    "align-",
    "appearance-",
    "jusitfy",
    "animate-",
    "backdrop-",
    "blur",
    "brightness",
    "divide-",
    "from-",
    "to-",
    "via-",
    "line-clamp",
    "mix-",
    "will-",
];

/// A utility class (Tailwind-style, or an obfuscated one with digits) says how an element
/// looks, not what it is, so it never names a component.
pub fn is_utility_class(c: &str) -> bool {
    c.contains([':', '[', ']', '/', '(', ')', '%', '!', '@', '.'])
        || c.starts_with('-')
        || c.chars().any(|ch| ch.is_ascii_digit())
        || UTILITY_PREFIXES
            .iter()
            .any(|p| c == p.trim_end_matches('-') || c.starts_with(p))
}

/// A BEM modifier (`block--open`, `is-active`) is state, not structure.
pub fn is_state_class(c: &str) -> bool {
    c.contains("--") || c.starts_with("is-") || c.starts_with("has-")
}

fn significant_children(doc: &Doc, id: NodeId) -> Vec<NodeId> {
    doc.node(id)
        .children
        .iter()
        .copied()
        .filter(|c| {
            let n = doc.node(*c);
            match &n.text {
                Some(t) => !t.trim().is_empty(),
                None => !matches!(n.tag.as_str(), "script" | "style" | "template"),
            }
        })
        .collect()
}

/// Structural hash of the subtree rooted at `id`.
/// Subtrees deeper than this hash as one constant (recursion stays bounded).
const MAX_HASH_DEPTH: usize = 256;

/// Hash of one node from its own facts and the hashes of its significant children.
fn combine(doc: &Doc, id: NodeId, child: &dyn Fn(NodeId) -> u64) -> u64 {
    let n = doc.node(id);
    if n.text.is_some() {
        return mix(FNV_OFFSET, b"#text");
    }
    let mut h = mix(FNV_OFFSET, n.tag.as_bytes());
    h = mix(h, n.role().unwrap_or("").as_bytes());
    let mut classes: Vec<&str> = n
        .classes
        .iter()
        .map(String::as_str)
        .filter(|c| !is_state_class(c))
        .collect();
    classes.sort_unstable();
    for c in classes {
        h = mix(h, c.as_bytes());
    }
    for name in n
        .attrs
        .keys()
        .filter(|k| !STATE_ATTRS.contains(&k.as_str()))
    {
        h = mix(h, name.as_bytes());
    }
    // Adjacent text nodes collapse: `a`, `<b/>`, `c` hashes the same whatever the text.
    let mut last_text = false;
    for c in significant_children(doc, id) {
        let is_text = doc.node(c).text.is_some();
        if is_text && last_text {
            continue;
        }
        last_text = is_text;
        h = mix(h, &child(c).to_le_bytes());
    }
    h
}

/// Structural hash of the subtree rooted at `id`.
pub fn shape_hash(doc: &Doc, id: NodeId) -> u64 {
    fn go(doc: &Doc, id: NodeId, depth: usize) -> u64 {
        if depth > MAX_HASH_DEPTH {
            return mix(FNV_OFFSET, b"#deep");
        }
        combine(doc, id, &|c| go(doc, c, depth + 1))
    }
    go(doc, id, 0)
}

/// Hashes and element counts of every subtree, bottom-up without recursion. Children always
/// have larger ids than their parent, so one reverse pass sees every child first.
fn tables(doc: &Doc) -> (Vec<u64>, Vec<usize>) {
    let n = doc.nodes.len();
    let mut hashes = vec![0u64; n];
    let mut sizes = vec![0usize; n];
    for id in (0..n).rev() {
        let node = doc.node(id);
        if node.is_element() {
            hashes[id] = combine(doc, id, &|c| hashes[c]);
            sizes[id] = 1 + node.children.iter().map(|c| sizes[*c]).sum::<usize>();
        } else {
            hashes[id] = mix(FNV_OFFSET, b"#text");
        }
    }
    (hashes, sizes)
}

fn subtree_size(doc: &Doc, id: NodeId) -> usize {
    1 + doc
        .descendants(id)
        .into_iter()
        .filter(|d| doc.node(*d).is_element())
        .count()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotKind {
    Text,
    Attribute,
    Variant,
}

/// A position where instances differ.
#[derive(Debug, Clone, Serialize)]
pub struct Slot {
    pub kind: SlotKind,
    /// Child-index path from the instance root.
    pub path: Vec<usize>,
    /// Attribute name for `Attribute` slots.
    pub attr: Option<String>,
    /// Value per instance, in instance order.
    pub values: Vec<String>,
    /// Derived prop name; `None` when no semantic name could be derived.
    pub name: Option<String>,
    pub name_source: Option<String>,
    pub ts_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub shape: u64,
    pub instances: Vec<NodeId>,
    pub size: usize,
    pub component_name: Option<String>,
    pub name_source: Option<String>,
    pub slots: Vec<Slot>,
    /// `true` when the component has a name and every varying slot has a unique prop name.
    pub extractable: bool,
    pub gaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Variant {
    pub a: NodeId,
    pub b: NodeId,
    pub distance: usize,
    pub size: usize,
    pub edits: Vec<TreeEdit>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Components {
    pub groups: Vec<Group>,
    pub variants: Vec<Variant>,
    /// The near-duplicate search hit its sibling or pair cap and did not compare everything.
    pub variants_truncated: bool,
}

/// Walk instances in lock step; every slot where values differ is recorded.
fn collect_slots(doc: &Doc, instances: &[NodeId]) -> Vec<Slot> {
    let mut slots = Vec::new();
    walk(doc, instances, &mut Vec::new(), &mut slots);
    slots
}

fn walk(doc: &Doc, nodes: &[NodeId], path: &mut Vec<usize>, out: &mut Vec<Slot>) {
    if path.len() > MAX_HASH_DEPTH {
        return;
    }
    let first = doc.node(nodes[0]);
    if first.text.is_some() {
        let values: Vec<String> = nodes
            .iter()
            .map(|n| {
                doc.node(*n)
                    .text
                    .clone()
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            })
            .collect();
        if values.iter().any(|v| v != &values[0]) {
            out.push(Slot {
                kind: SlotKind::Text,
                path: path.clone(),
                attr: None,
                values,
                name: None,
                name_source: None,
                ts_type: "string".into(),
            });
        }
        return;
    }
    // Attributes: same names by construction (the shape hash includes them).
    for name in first.attrs.keys().filter(|k| {
        !matches!(
            k.as_str(),
            "id" | "style" | "class" | "key" | "aria-controls" | "aria-labelledby"
        )
    }) {
        let values: Vec<String> = nodes
            .iter()
            .map(|n| doc.node(*n).attr(name).unwrap_or_default().to_owned())
            .collect();
        if values.iter().any(|v| v != &values[0]) {
            let state_like = STATE_ATTRS.contains(&name.as_str());
            out.push(Slot {
                kind: if state_like {
                    SlotKind::Variant
                } else {
                    SlotKind::Attribute
                },
                path: path.clone(),
                attr: Some(name.clone()),
                values,
                name: None,
                name_source: None,
                ts_type: "string".into(),
            });
        }
    }
    // Class modifiers are variants.
    let modifiers: Vec<Vec<String>> = nodes
        .iter()
        .map(|n| {
            let mut m: Vec<String> = doc
                .node(*n)
                .classes
                .iter()
                .filter(|c| is_state_class(c))
                .cloned()
                .collect();
            m.sort();
            m
        })
        .collect();
    if modifiers.iter().any(|m| m != &modifiers[0]) {
        out.push(Slot {
            kind: SlotKind::Variant,
            path: path.clone(),
            attr: Some("class".into()),
            values: modifiers.iter().map(|m| m.join(" ")).collect(),
            name: None,
            name_source: None,
            ts_type: "string".into(),
        });
    }
    let kids: Vec<Vec<NodeId>> = nodes
        .iter()
        .map(|n| collapse_text(doc, significant_children(doc, *n)))
        .collect();
    for i in 0..kids[0].len() {
        path.push(i);
        let column: Vec<NodeId> = kids.iter().filter_map(|k| k.get(i).copied()).collect();
        if column.len() == nodes.len() {
            walk(doc, &column, path, out);
        }
        path.pop();
    }
}

fn collapse_text(doc: &Doc, kids: Vec<NodeId>) -> Vec<NodeId> {
    let mut out: Vec<NodeId> = Vec::new();
    let mut last_text = false;
    for k in kids {
        let t = doc.node(k).text.is_some();
        if t && last_text {
            continue;
        }
        last_text = t;
        out.push(k);
    }
    out
}

/// Detect repeated structures in `doc`.
pub fn extract(doc: &Doc) -> Components {
    let (hashes, sizes) = tables(doc);
    let mut by_shape: BTreeMap<u64, Vec<NodeId>> = BTreeMap::new();
    for n in &doc.nodes {
        if !n.is_element()
            || matches!(
                n.tag.as_str(),
                "html" | "head" | "body" | "script" | "style" | "svg" | "path"
            )
        {
            continue;
        }
        // Inside SVG nothing is a component: geometry belongs to the SVG process.
        if doc.ancestors(n.id).any(|a| doc.node(a).tag == "svg") {
            continue;
        }
        if sizes[n.id] < 3 {
            continue;
        }
        by_shape.entry(hashes[n.id]).or_default().push(n.id);
    }
    let mut groups: Vec<Group> = Vec::new();
    for (shape, mut instances) in by_shape {
        if instances.len() < 2 {
            continue;
        }
        instances.sort_unstable();
        // Instances must not nest in each other.
        if instances
            .iter()
            .any(|a| instances.iter().any(|b| a != b && doc.is_ancestor(*a, *b)))
        {
            continue;
        }
        let size = subtree_size(doc, instances[0]);
        groups.push(Group {
            shape,
            instances,
            size,
            component_name: None,
            name_source: None,
            slots: Vec::new(),
            extractable: false,
            gaps: Vec::new(),
        });
    }
    // Keep maximal groups: drop a group whose every instance sits inside an instance
    // of a larger group with the same number of instances.
    let snapshot = groups.clone();
    groups.retain(|g| {
        !snapshot.iter().any(|big| {
            big.size > g.size
                && big.instances.len() == g.instances.len()
                && g.instances
                    .iter()
                    .all(|i| big.instances.iter().any(|b| doc.is_ancestor(*b, *i)))
        })
    });
    for g in &mut groups {
        g.slots = collect_slots(doc, &g.instances);
        naming::name_group(doc, g);
    }
    groups.sort_by_key(|g| (g.instances[0], g.shape));
    let (variants, variants_truncated) = near_duplicates(doc, &groups);
    Components {
        groups,
        variants,
        variants_truncated,
    }
}

/// Subtrees under one parent that differ by a few nodes: small tree-edit distance.
/// Pairs compared per parent and in total: tree edit distance is cubic-ish, so near-duplicate
/// search is bounded and says so (`variants_truncated`) rather than running unbounded.
const MAX_SIBLINGS: usize = 24;
const MAX_PAIRS: usize = 4000;

fn near_duplicates(doc: &Doc, groups: &[Group]) -> (Vec<Variant>, bool) {
    let grouped: std::collections::BTreeSet<NodeId> = groups
        .iter()
        .flat_map(|g| g.instances.iter().copied())
        .collect();
    let (_, sizes) = tables(doc);
    let mut out = Vec::new();
    let mut pairs = 0usize;
    let mut truncated = false;
    for parent in &doc.nodes {
        if !parent.is_element() {
            continue;
        }
        let all: Vec<NodeId> = doc
            .element_children(parent.id)
            .filter(|c| !grouped.contains(c) && (4..=120).contains(&sizes[*c]))
            .collect();
        truncated |= all.len() > MAX_SIBLINGS;
        let kids: Vec<NodeId> = all.into_iter().take(MAX_SIBLINGS).collect();
        for (i, a) in kids.iter().enumerate() {
            for b in &kids[i + 1..] {
                if doc.node(*a).tag != doc.node(*b).tag {
                    continue;
                }
                let size = sizes[*a].max(sizes[*b]);
                // Trees whose sizes differ by more than a quarter cannot be that close.
                if sizes[*a].abs_diff(sizes[*b]) * 4 > size {
                    continue;
                }
                pairs += 1;
                if pairs > MAX_PAIRS {
                    return (out, true);
                }
                let (distance, edits) = tree_edit_distance(doc, *a, *b);
                if distance > 0 && distance * 4 <= size {
                    out.push(Variant {
                        a: *a,
                        b: *b,
                        distance,
                        size,
                        edits,
                    });
                }
            }
        }
    }
    (out, truncated)
}
