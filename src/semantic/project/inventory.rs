//! What the IR could and could not map, per route: elements rendered by compositions or
//! repeated-structure components versus plain markup, controls by primitive and unrecognized,
//! landmarks left as plain markup, and the graphics, forms and scripts that belong to later
//! milestones. Counts only; nothing here is a fidelity claim.
use crate::semantic::PageAnalysis;
use crate::semantic::emit::Emitted;
use crate::semantic::patterns::Primitive;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Counts {
    pub total: usize,
    pub mapped: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RouteInventory {
    pub route: String,
    pub viewport: Option<(u32, u32)>,
    /// Elements in `<body>`.
    pub body_elements: usize,
    /// Elements rendered by a Base UI composition or a repeated-structure component.
    pub component_elements: usize,
    /// Elements left as plain markup.
    pub plain_elements: usize,
    pub controls: usize,
    pub controls_by_primitive: BTreeMap<Primitive, usize>,
    pub native_links: usize,
    pub unrecognized_controls: usize,
    /// Landmark and section elements: `total` and how many sit inside a component (`mapped`).
    pub landmarks: BTreeMap<String, Counts>,
    /// Graphics and embeds whose conversion belongs to the canvas/shader/asset milestones.
    pub graphics: BTreeMap<String, usize>,
    pub forms: BTreeMap<String, usize>,
    pub scripts_omitted: usize,
    /// Elements deliberately not emitted, by reason (scripts, framework runtime, `javascript:` embeds).
    pub elements_dropped: BTreeMap<String, usize>,
    /// Elements of the body that carry an inline `style` attribute in the capture: none is emitted.
    pub inline_style_attributes_in_capture: usize,
    /// Same-origin links whose target path is not an assembled route (404 in the project).
    pub links_to_uncaptured_routes: Vec<String>,
    pub custom_elements: Vec<String>,
    pub patterns_recognized: BTreeMap<Primitive, usize>,
    pub patterns_without_composition: usize,
    pub patterns_not_placed: Vec<(usize, String)>,
    pub repeated_groups: usize,
    pub repeated_groups_extracted: usize,
    pub near_duplicate_pairs: usize,
}

const LANDMARKS: &[&str] = &[
    "header", "nav", "main", "footer", "aside", "section", "article",
];
const GRAPHICS: &[&str] = &[
    "canvas", "video", "audio", "iframe", "object", "embed", "picture", "svg",
];
const FORMS: &[&str] = &["form", "input", "textarea", "select", "button"];

pub fn route(route: &str, a: &PageAnalysis, emitted: &Emitted) -> RouteInventory {
    let doc = &a.doc;
    let page = emitted.page.clone().unwrap_or_default();
    let body = doc.nodes.iter().find(|n| n.tag == "body").map(|n| n.id);
    let in_body: Vec<_> = body
        .map(|b| {
            doc.descendants(b)
                .into_iter()
                .filter(|d| doc.node(*d).is_element())
                .collect()
        })
        .unwrap_or_default();
    let mut inv = RouteInventory {
        route: route.to_owned(),
        viewport: doc.viewport,
        body_elements: page.elements,
        component_elements: page.covered_elements,
        plain_elements: page.raw_elements,
        controls: a.recognition.controls,
        controls_by_primitive: a.recognition.controls_by_primitive.clone(),
        native_links: a.recognition.native,
        unrecognized_controls: a.recognition.unrecognized.len(),
        scripts_omitted: page.scripts_omitted,
        elements_dropped: page.elements_dropped.clone(),
        custom_elements: emitted.custom_elements.iter().cloned().collect(),
        patterns_not_placed: page.unplaced.clone(),
        repeated_groups: a.components.groups.len(),
        repeated_groups_extracted: emitted.group_components.len(),
        near_duplicate_pairs: a.components.variants.len(),
        ..Default::default()
    };
    for d in in_body {
        let node = doc.node(d);
        let tag = node.tag.as_str();
        if LANDMARKS.contains(&tag) {
            let c = inv.landmarks.entry(tag.to_owned()).or_default();
            c.total += 1;
            if page.covered_nodes.contains(&d) {
                c.mapped += 1;
            }
        }
        if GRAPHICS.contains(&tag) && !doc.ancestors(d).any(|x| doc.node(x).tag == "svg") {
            *inv.graphics.entry(tag.to_owned()).or_default() += 1;
        }
        if FORMS.contains(&tag) {
            *inv.forms.entry(tag.to_owned()).or_default() += 1;
        }
    }
    for m in &a.recognition.matches {
        if m.primitive != Primitive::Button {
            *inv.patterns_recognized.entry(m.primitive).or_default() += 1;
        }
    }
    inv.inline_style_attributes_in_capture = doc
        .descendants(body.unwrap_or(0))
        .into_iter()
        .filter(|d| doc.node(*d).has_attr("style"))
        .count();
    inv.patterns_without_composition = emitted
        .skipped
        .iter()
        .filter(|s| s.primitive != Primitive::Button)
        .count();
    inv
}
