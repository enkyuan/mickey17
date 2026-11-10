//! Build an IR [`Doc`] from a saved capture, or from an HTML string (fixtures,
//! static sources). Both go through the same role/name resolution.
use super::{Doc, Node, NodeId, Rect, aria};
use crate::evidence::{Capture, Content};
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, HashMap};

/// Computed properties kept on IR nodes: visibility, layering, and the cues that
/// distinguish modal surfaces and clickable non-semantic elements.
pub const STYLE_KEYS: &[&str] = &[
    "display",
    "visibility",
    "position",
    "overflow-x",
    "overflow-y",
    "opacity",
    "pointer-events",
    "cursor",
    "z-index",
    "transform",
    "inset",
    "top",
    "left",
    "right",
    "bottom",
    "width",
    "height",
    "flex-direction",
    "grid-template-columns",
    "transition-property",
    "transition-duration",
    "animation-name",
    "background-color",
    "color",
    "font-family",
    "font-size",
    "font-weight",
    "line-height",
    "letter-spacing",
    "border-radius",
    "box-shadow",
    "padding",
    "gap",
];

impl Doc {
    /// Build from one captured page. Node ids are the capture's node indices.
    pub fn from_capture(capture: &Capture) -> Result<Doc> {
        let count = capture.nodes.len();
        let mut nodes: Vec<Node> = Vec::with_capacity(count);
        for (id, raw) in capture.nodes.iter().enumerate() {
            ensure!(
                raw.parent.map(|p| p < id).unwrap_or(true),
                "capture node {id} does not follow its parent"
            );
            nodes.push(match &raw.content {
                Content::Text { value } => Node {
                    id,
                    parent: raw.parent,
                    children: Vec::new(),
                    tag: String::new(),
                    text: Some(value.clone()),
                    attrs: BTreeMap::new(),
                    classes: Vec::new(),
                    style: BTreeMap::new(),
                    bounds: None,
                    role: None,
                    role_source: None,
                    name: None,
                    name_source: None,
                },
                Content::Element {
                    tag,
                    attributes,
                    style,
                    bounds,
                    ..
                } => {
                    let attrs: BTreeMap<String, String> = attributes
                        .iter()
                        .map(|a| (a.name.clone(), a.value.clone()))
                        .collect();
                    let classes = classes_of(&attrs);
                    let computed = capture.styles.get(*style);
                    let style = STYLE_KEYS
                        .iter()
                        .filter_map(|k| {
                            computed
                                .and_then(|m| m.get(*k))
                                .map(|v| ((*k).to_owned(), v.clone()))
                        })
                        .collect();
                    Node {
                        id,
                        parent: raw.parent,
                        children: Vec::new(),
                        tag: tag.clone(),
                        text: None,
                        attrs,
                        classes,
                        style,
                        bounds: Some(Rect {
                            x: bounds.x,
                            y: bounds.y,
                            width: bounds.width,
                            height: bounds.height,
                        }),
                        role: None,
                        role_source: None,
                        name: None,
                        name_source: None,
                    }
                }
            });
        }
        let mut doc = Doc {
            url: capture.url.clone(),
            title: capture.metadata.title.clone(),
            viewport: Some((capture.viewport.width, capture.viewport.height)),
            nodes,
            by_id: BTreeMap::new(),
        };
        doc.finish();
        Ok(doc)
    }

    /// Build from an HTML document or fragment. No computed style is available.
    pub fn from_html(html: &str) -> Doc {
        let parsed = scraper::Html::parse_document(html);
        let mut ids: HashMap<_, NodeId> = HashMap::new();
        let mut nodes: Vec<Node> = Vec::new();
        for node in parsed.tree.root().descendants() {
            let (tag, text, attrs) = match node.value() {
                scraper::Node::Element(el) => (
                    el.name().to_owned(),
                    None,
                    el.attrs()
                        .map(|(k, v)| (k.to_owned(), v.to_owned()))
                        .collect::<BTreeMap<_, _>>(),
                ),
                scraper::Node::Text(t) => (String::new(), Some(t.to_string()), BTreeMap::new()),
                _ => continue,
            };
            let parent = node.ancestors().find_map(|a| ids.get(&a.id()).copied());
            let id = nodes.len();
            ids.insert(node.id(), id);
            let classes = classes_of(&attrs);
            nodes.push(Node {
                id,
                parent,
                children: Vec::new(),
                tag,
                text,
                attrs,
                classes,
                style: BTreeMap::new(),
                bounds: None,
                role: None,
                role_source: None,
                name: None,
                name_source: None,
            });
        }
        let mut doc = Doc {
            url: String::new(),
            title: String::new(),
            viewport: None,
            nodes,
            by_id: BTreeMap::new(),
        };
        doc.finish();
        if let Some(t) = doc.nodes.iter().find(|n| n.tag == "title").map(|n| n.id) {
            doc.title = doc.text_content(t, 200);
        }
        doc
    }

    fn finish(&mut self) {
        for i in 0..self.nodes.len() {
            if let Some(p) = self.nodes[i].parent {
                self.nodes[p].children.push(i);
            }
        }
        for n in &self.nodes {
            if let Some(id) = n.attr("id")
                && !id.is_empty()
            {
                self.by_id.entry(id.to_owned()).or_insert(n.id);
            }
        }
        aria::resolve_roles(self);
        aria::resolve_names(self);
    }
}

fn classes_of(attrs: &BTreeMap<String, String>) -> Vec<String> {
    attrs
        .get("class")
        .map(|c| c.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}
