//! Effective role and accessible name, following the HTML-AAM / accname
//! algorithms closely enough for pattern recognition. Departures are listed in
//! `docs/SEMANTIC.md` ("Known gaps"): no CSS generated content, no `aria-describedby`,
//! name-from-content is flattened text.
use super::{Doc, NameSource, NodeId, RoleSource};

const INTERACTIVE: &[&str] = &[
    "button",
    "link",
    "tab",
    "checkbox",
    "radio",
    "switch",
    "textbox",
    "searchbox",
    "combobox",
    "listbox",
    "option",
    "slider",
    "spinbutton",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "treeitem",
    "gridcell",
];

pub fn is_interactive_role(role: &str) -> bool {
    INTERACTIVE.contains(&role)
}

const ABSTRACT_OR_UNKNOWN_SKIP: &[&str] = &["none", "presentation"];

/// Roles whose accessible name may come from their content.
const NAME_FROM_CONTENT: &[&str] = &[
    "button",
    "link",
    "tab",
    "checkbox",
    "radio",
    "switch",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "heading",
    "treeitem",
    "tooltip",
    "cell",
    "columnheader",
    "rowheader",
    "gridcell",
    "listitem",
];

/// Implicit ARIA role of an element, from its tag and a few attributes.
pub fn implicit_role(
    tag: &str,
    attr: &dyn Fn(&str) -> Option<String>,
    inside_sectioning: bool,
) -> Option<&'static str> {
    let has = |n: &str| attr(n).is_some();
    Some(match tag {
        "a" | "area" if has("href") => "link",
        "button" => "button",
        "summary" => "button",
        "nav" => "navigation",
        "main" => "main",
        "aside" => "complementary",
        "header" if !inside_sectioning => "banner",
        "footer" if !inside_sectioning => "contentinfo",
        "form" => "form",
        "search" => "search",
        "section" if has("aria-label") || has("aria-labelledby") => "region",
        "article" => "article",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "ul" | "ol" | "menu" => "list",
        "li" => "listitem",
        "dl" => return None,
        "dt" => "term",
        "dd" => "definition",
        "table" => "table",
        "tr" => "row",
        "td" => "cell",
        "th" => "columnheader",
        "img" => match attr("alt") {
            Some(a) if a.is_empty() => "presentation",
            _ => "img",
        },
        "hr" => "separator",
        "dialog" => "dialog",
        "progress" => "progressbar",
        "meter" => "meter",
        "output" => "status",
        "fieldset" => "group",
        "details" => "group",
        "textarea" => "textbox",
        "select" => {
            if has("multiple")
                || attr("size")
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0)
                    > 1
            {
                "listbox"
            } else {
                "combobox"
            }
        }
        "option" => "option",
        "input" => {
            let kind = attr("type").unwrap_or_default().to_ascii_lowercase();
            match kind.as_str() {
                "checkbox" => "checkbox",
                "radio" => "radio",
                "range" => "slider",
                "number" => "spinbutton",
                "search" if has("list") => "combobox",
                "search" => "searchbox",
                "button" | "submit" | "reset" | "image" => "button",
                "hidden" => return None,
                "text" | "email" | "tel" | "url" | "" if has("list") => "combobox",
                _ => "textbox",
            }
        }
        _ => return None,
    })
}

pub(super) fn resolve_roles(doc: &mut Doc) {
    for i in 0..doc.nodes.len() {
        if !doc.nodes[i].is_element() {
            continue;
        }
        let explicit = doc.nodes[i]
            .attr("role")
            .and_then(|r| r.split_whitespace().next())
            .map(str::to_ascii_lowercase);
        let (role, source) = match explicit {
            Some(role) if !ABSTRACT_OR_UNKNOWN_SKIP.contains(&role.as_str()) => {
                (Some(role), Some(RoleSource::Explicit))
            }
            Some(_) => (None, None),
            None => {
                let inside = doc.ancestors(i).any(|a| {
                    matches!(
                        doc.nodes[a].tag.as_str(),
                        "article" | "aside" | "main" | "nav" | "section"
                    )
                });
                let node = &doc.nodes[i];
                let role = implicit_role(&node.tag, &|name| node.attrs.get(name).cloned(), inside);
                (role.map(str::to_owned), role.map(|_| RoleSource::Implicit))
            }
        };
        doc.nodes[i].role = role;
        doc.nodes[i].role_source = source;
    }
}

/// Accessible names. `aria-labelledby` > `aria-label` > native label > content > title.
pub(super) fn resolve_names(doc: &mut Doc) {
    let mut labels_for: std::collections::BTreeMap<String, Vec<NodeId>> = Default::default();
    for n in &doc.nodes {
        if n.tag == "label"
            && let Some(target) = n.attr("for")
        {
            labels_for.entry(target.to_owned()).or_default().push(n.id);
        }
    }
    for i in 0..doc.nodes.len() {
        if !doc.nodes[i].is_element() || doc.nodes[i].role.is_none() {
            continue;
        }
        let role = doc.nodes[i].role.clone().unwrap_or_default();
        let resolved = name_of(doc, i, &role, &labels_for);
        if let Some((name, source)) = resolved {
            doc.nodes[i].name = Some(name);
            doc.nodes[i].name_source = Some(source);
        }
    }
}

fn clean(s: String) -> Option<String> {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    (!s.is_empty()).then_some(s)
}

fn name_of(
    doc: &Doc,
    id: NodeId,
    role: &str,
    labels_for: &std::collections::BTreeMap<String, Vec<NodeId>>,
) -> Option<(String, NameSource)> {
    let n = &doc.nodes[id];
    let by = n.id_refs("aria-labelledby");
    if !by.is_empty() {
        let text: Vec<String> = by
            .iter()
            .filter_map(|r| doc.resolve(r))
            .map(|t| doc.text_content(t, 200))
            .collect();
        if let Some(name) = clean(text.join(" ")) {
            return Some((name, NameSource::LabelledBy));
        }
    }
    if let Some(name) = n.attr("aria-label").map(str::to_owned).and_then(clean) {
        return Some((name, NameSource::AriaLabel));
    }
    // Native labelling: <label for>, wrapping <label>, <fieldset>/<legend>, img alt.
    if matches!(
        n.tag.as_str(),
        "input" | "select" | "textarea" | "meter" | "progress"
    ) {
        if let Some(dom_id) = n.attr("id")
            && let Some(labels) = labels_for.get(dom_id)
        {
            let text: Vec<String> = labels.iter().map(|l| doc.text_content(*l, 200)).collect();
            if let Some(name) = clean(text.join(" ")) {
                return Some((name, NameSource::NativeLabel));
            }
        }
        if let Some(label) = doc.ancestors(id).find(|a| doc.nodes[*a].tag == "label")
            && let Some(name) = clean(doc.text_content(label, 200))
        {
            return Some((name, NameSource::NativeLabel));
        }
    }
    if n.tag == "img"
        && let Some(name) = n.attr("alt").map(str::to_owned).and_then(clean)
    {
        return Some((name, NameSource::NativeLabel));
    }
    if n.tag == "fieldset"
        && let Some(legend) = doc
            .element_children(id)
            .find(|c| doc.nodes[*c].tag == "legend")
        && let Some(name) = clean(doc.text_content(legend, 200))
    {
        return Some((name, NameSource::NativeLabel));
    }
    if NAME_FROM_CONTENT.contains(&role) {
        if let Some(name) = clean(doc.text_content(id, 200)) {
            return Some((name, NameSource::Content));
        }
        // SVG <title> as the only content.
        for d in doc.descendants(id) {
            if doc.nodes[d].tag == "title"
                && let Some(name) = clean(doc.text_content(d, 200))
            {
                return Some((name, NameSource::Content));
            }
        }
    }
    if (n.tag == "svg" || role == "img")
        && let Some(t) = doc
            .descendants(id)
            .into_iter()
            .find(|d| doc.nodes[*d].tag == "title")
        && let Some(name) = clean(doc.text_content(t, 200))
    {
        return Some((name, NameSource::Content));
    }
    if let Some(name) = n.attr("title").map(str::to_owned).and_then(clean) {
        return Some((name, NameSource::Title));
    }
    n.attr("placeholder")
        .map(str::to_owned)
        .and_then(clean)
        .map(|p| (p, NameSource::Placeholder))
}
