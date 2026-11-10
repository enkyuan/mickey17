//! Semantic component and prop names. Sources, in order: `data-component`,
//! BEM class blocks/elements, roles and heading levels, tag semantics, and the
//! nearest named ancestor. A name nothing supports is `None`; no `ComponentN`,
//! `prop_N` or hash-derived names are ever produced.
use super::{Group, Slot, SlotKind, is_state_class, is_utility_class};
use crate::semantic::ir::{Doc, NodeId};
use std::collections::BTreeSet;

/// Split an identifier-ish string into lowercase words: separators, camelCase humps
/// and acronym boundaries (`APIExplorer` -> `api`, `explorer`).
pub fn words(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1).copied();
        let hump = c.is_uppercase()
            && !cur.is_empty()
            && (prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit())
                || (prev.is_some_and(char::is_uppercase) && next.is_some_and(char::is_lowercase)));
        if hump {
            out.push(std::mem::take(&mut cur));
        }
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn pascal(s: &str) -> String {
    words(s)
        .iter()
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect()
}

/// A PascalCase identifier is kept verbatim (`APIExplorer`); anything else is converted.
pub fn identifier_or_pascal(s: &str) -> String {
    let ident =
        s.chars().next().is_some_and(char::is_uppercase) && s.chars().all(char::is_alphanumeric);
    if ident { s.to_owned() } else { pascal(s) }
}

pub fn camel(s: &str) -> String {
    let p = pascal(s);
    let mut c = p.chars();
    let name = c
        .next()
        .map(|f| f.to_lowercase().collect::<String>() + c.as_str())
        .unwrap_or_default();
    // A leading digit is not a valid identifier start.
    if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        String::new()
    } else {
        name
    }
}

/// ECMAScript reserved and strict-mode words, plus names that clash with React/JSX props or
/// globals a prop would shadow.
const RESERVED: &[&str] = &[
    "arguments",
    "await",
    "break",
    "case",
    "catch",
    "children",
    "class",
    "const",
    "constructor",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "key",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "ref",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "NaN",
    "Infinity",
];

fn bem_element(class: &str) -> Option<&str> {
    class
        .split_once("__")
        .map(|(_, e)| e.split("--").next().unwrap_or(e))
        .filter(|e| !e.is_empty())
}

fn common_classes(doc: &Doc, instances: &[NodeId]) -> Vec<String> {
    let first: BTreeSet<&String> = doc
        .node(instances[0])
        .classes
        .iter()
        .filter(|c| !is_state_class(c) && !is_utility_class(c))
        .collect();
    first
        .into_iter()
        .filter(|c| instances.iter().all(|i| doc.node(*i).has_class(c)))
        .cloned()
        .collect()
}

fn tag_noun(tag: &str, role: Option<&str>) -> Option<&'static str> {
    Some(match (tag, role) {
        ("li", _) => "Item",
        ("article", _) => "Card",
        ("tr", _) => "Row",
        ("dt" | "dd", _) => "Entry",
        ("figure", _) => "Figure",
        ("section", _) => "Section",
        ("a", _) => "Link",
        ("button", _) => "Button",
        _ => return None,
    })
}

fn name_component(doc: &Doc, instances: &[NodeId]) -> Option<(String, String)> {
    let dc: Vec<&str> = instances
        .iter()
        .map(|i| doc.node(*i).attr("data-component").unwrap_or(""))
        .collect();
    if !dc[0].is_empty() && dc.iter().all(|d| *d == dc[0]) {
        return Some((
            identifier_or_pascal(dc[0]),
            format!("data-component=\"{}\"", dc[0]),
        ));
    }
    let common = common_classes(doc, instances);
    // Prefer a `block__element` class: its full name is the most specific.
    if let Some(c) = common
        .iter()
        .filter(|c| c.contains("__"))
        .max_by_key(|c| c.len())
    {
        return Some((pascal(c), format!("class `{c}`")));
    }
    if let Some(c) = common.iter().max_by_key(|c| c.len())
        && c.len() > 3
    {
        return Some((pascal(c), format!("class `{c}`")));
    }
    // Nearest ancestor with a data-component / BEM block, plus the element's noun.
    let first = doc.node(instances[0]);
    let noun = tag_noun(&first.tag, first.role())?;
    let owner = doc.ancestors(instances[0]).find_map(|a| {
        let n = doc.node(a);
        n.attr("data-component")
            .map(|d| (d.to_owned(), "data-component"))
    })?;
    Some((
        format!("{}{}", pascal(&owner.0), noun),
        format!("ancestor {} `{}` + <{}>", owner.1, owner.0, first.tag),
    ))
}

pub fn element_at(doc: &Doc, instance: NodeId, path: &[usize]) -> NodeId {
    let mut cur = instance;
    for &i in path {
        let kids = super::collapse_text(doc, super::significant_children(doc, cur));
        cur = kids[i];
    }
    cur
}

fn name_slot(
    doc: &Doc,
    instance: NodeId,
    slot: &Slot,
    component: Option<&str>,
) -> Option<(String, String)> {
    let at = element_at(doc, instance, &slot.path);
    // A text slot belongs to the element holding the text.
    let el = if slot.kind == SlotKind::Text {
        doc.node(at).parent?
    } else {
        at
    };
    let n = doc.node(el);
    match (slot.kind, slot.attr.as_deref()) {
        (SlotKind::Attribute, Some(attr)) => {
            let name = match attr {
                "href" => "href".to_owned(),
                "src" => "src".to_owned(),
                "alt" => "alt".to_owned(),
                "srcset" => "srcSet".to_owned(),
                "aria-label" => "label".to_owned(),
                "title" => "title".to_owned(),
                a if a.starts_with("data-") => camel(&a[5..]),
                a => camel(a),
            };
            (!name.is_empty()).then(|| (name, format!("attribute `{attr}`")))
        }
        (SlotKind::Variant, Some("class")) => Some(("variant".into(), "class modifier".into())),
        (SlotKind::Text, _) => {
            if matches!(n.tag.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                return Some(("title".into(), format!("text of <{}>", n.tag)));
            }
            if n.tag == "time" {
                return Some(("date".into(), "text of <time>".into()));
            }
            // BEM element: `card__stat-label` -> statLabel.
            if let Some(e) = n
                .classes
                .iter()
                .filter(|c| !is_state_class(c) && !is_utility_class(c))
                .find_map(|c| bem_element(c))
            {
                let name = camel(e);
                if !name.is_empty() {
                    return Some((name, format!("BEM element `{e}`")));
                }
            }
            // The only text of a link or button labels it; a card-sized link with several
            // text runs does not (its headings and paragraphs are named by what they are).
            let labelled = std::iter::once(el)
                .chain(
                    doc.ancestors(el)
                        .take_while(|a| *a != instance)
                        .chain(std::iter::once(instance)),
                )
                .find(|a| matches!(doc.node(*a).role(), Some("link" | "button")))
                .filter(|owner| {
                    doc.descendants(*owner)
                        .into_iter()
                        .filter(|d| {
                            doc.node(*d)
                                .text
                                .as_deref()
                                .is_some_and(|t| !t.trim().is_empty())
                        })
                        .count()
                        == 1
                });
            if let Some(owner) = labelled {
                return Some((
                    "label".into(),
                    format!(
                        "only text inside <{}> ({})",
                        doc.node(owner).tag,
                        doc.node(owner).role().unwrap_or("")
                    ),
                ));
            }
            if n.tag == "p" {
                return Some(("description".into(), "text of <p>".into()));
            }
            if n.role() == Some("heading") {
                return Some(("title".into(), "text of a heading role".into()));
            }
            // A single non-modifier class names the element.
            let cls: Vec<&String> = n
                .classes
                .iter()
                .filter(|c| !is_state_class(c) && !is_utility_class(c))
                .collect();
            if cls.len() == 1 {
                let mut name = camel(cls[0]);
                // `blog-item-title` inside `BlogItem` is just `title`.
                if let Some(comp) = component {
                    let prefix = camel(comp);
                    if name.len() > prefix.len() && name.starts_with(&prefix) {
                        name = camel(&name[prefix.len()..]);
                    }
                }
                if !name.is_empty() {
                    return Some((name, format!("class `{}`", cls[0])));
                }
            }
            None
        }
        _ => None,
    }
}

pub(super) fn name_group(doc: &Doc, g: &mut Group) {
    if let Some((name, source)) = name_component(doc, &g.instances) {
        g.component_name = Some(name);
        g.name_source = Some(source);
    } else {
        g.gaps.push(
            "no semantic component name: no shared data-component, class block, or named ancestor"
                .into(),
        );
    }
    // State attributes (aria-expanded, ...) differ between instances but are state, not props.
    g.slots.retain(|s| {
        !(s.kind == SlotKind::Variant && s.attr.as_deref().is_some_and(|a| a != "class"))
    });
    for s in &mut g.slots {
        let named = name_slot(doc, g.instances[0], s, g.component_name.as_deref());
        if let Some((n, src)) = named
            && !RESERVED.contains(&n.as_str())
        {
            s.name = Some(n);
            s.name_source = Some(src);
        }
        // `number` only when every value prints back exactly as captured ("01" and "9.90" do not).
        let numeric = s
            .values
            .iter()
            .all(|v| v.parse::<f64>().is_ok_and(|n| n.to_string() == *v));
        if numeric && s.kind != SlotKind::Variant {
            s.ts_type = "number".into();
        }
    }
    // Names must be unique; a duplicate gets its parent's name as a prefix, else stays unnamed.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut dup: Vec<usize> = Vec::new();
    for (i, s) in g.slots.iter().enumerate() {
        if let Some(n) = &s.name
            && !seen.insert(n.clone())
        {
            dup.push(i);
        }
    }
    for i in dup {
        let el = {
            let at = element_at(doc, g.instances[0], &g.slots[i].path);
            if g.slots[i].kind == SlotKind::Text {
                doc.node(at).parent
            } else {
                Some(at)
            }
        };
        let parent_name = el
            .and_then(|e| doc.node(e).parent)
            .and_then(|p| {
                doc.node(p)
                    .classes
                    .iter()
                    .find(|c| !is_state_class(c))
                    .and_then(|c| bem_element(c).map(camel))
            })
            .filter(|p| !p.is_empty());
        let current = g.slots[i].name.clone().unwrap();
        g.slots[i].name = match parent_name {
            Some(p) if !seen.contains(&camel(&format!("{p} {current}"))) => {
                let n = camel(&format!("{p} {current}"));
                seen.insert(n.clone());
                Some(n)
            }
            _ => None,
        };
    }
    let unnamed = g.slots.iter().filter(|s| s.name.is_none()).count();
    if unnamed > 0 {
        g.gaps.push(format!(
            "{unnamed} varying slot(s) have no derivable semantic prop name; the group is not extracted"
        ));
    }
    let variants = g
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Variant)
        .count();
    if variants > 0 {
        g.gaps.push(format!("{variants} slot(s) differ only by class modifier; variant props are not extracted yet, so the group is not extracted"));
    }
    g.extractable = g.component_name.is_some() && unnamed == 0 && variants == 0;
}
