//! The semantic report: every recognized pattern with its primitive and evidence,
//! every control no rule accepted, repeated-component groups, what the emitters
//! produced and skipped, and the gaps. Machine-readable JSON plus a Markdown view.
use super::PageAnalysis;
use super::emit::Emitted;
use super::ir::NodeId;
use super::patterns::Primitive;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Debug, Serialize)]
pub struct NodeRef {
    pub id: NodeId,
    pub tag: String,
    pub role: Option<String>,
    pub name: Option<String>,
    pub classes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub url: String,
    pub viewport: Option<(u32, u32)>,
    pub controls: usize,
    pub recognized_controls: usize,
    pub native_links: usize,
    pub unrecognized_controls: usize,
    pub by_primitive: BTreeMap<Primitive, usize>,
    /// Controls that belong to a Base UI primitive other than `Button`.
    pub non_button_primitive_controls: usize,
    pub patterns: usize,
    pub js_files_loaded: usize,
    pub js_files_with_parse_errors: Vec<String>,
    /// Scripts the page loaded from its own host that were not read, with the reason.
    pub js_files_skipped: Vec<(String, String)>,
    pub group_components: usize,
    pub near_duplicate_pairs: usize,
    pub near_duplicate_search_truncated: bool,
    pub ambiguous_script_links: usize,
    pub repeated_groups: usize,
    pub extractable_groups: usize,
    pub emitted_components: usize,
    pub skipped_patterns: usize,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub page: String,
    pub summary: Summary,
    pub matches: Vec<serde_json::Value>,
    pub unrecognized: Vec<serde_json::Value>,
    pub groups: Vec<serde_json::Value>,
    pub variants: Vec<serde_json::Value>,
    pub emitted: Emitted,
    /// Limits of this analysis that apply to every result above.
    pub limits: Vec<String>,
}

fn node_ref(a: &PageAnalysis, id: NodeId) -> NodeRef {
    let n = a.doc.node(id);
    NodeRef {
        id,
        tag: n.tag.clone(),
        role: n.role.clone(),
        name: n.name.clone(),
        classes: n.classes.clone(),
    }
}

pub fn build(a: &PageAnalysis, emitted: Emitted) -> Report {
    let r = &a.recognition;
    let recognized: usize = r.controls_by_primitive.values().sum();
    let non_button: usize = r
        .controls_by_primitive
        .iter()
        .filter(|(p, _)| **p != Primitive::Button)
        .map(|(_, n)| *n)
        .sum();
    let matches = r
        .matches
        .iter()
        .map(|m| {
            let mut v = serde_json::to_value(m).unwrap_or_default();
            v["root_ref"] = serde_json::to_value(node_ref(a, m.root)).unwrap_or_default();
            v
        })
        .collect();
    let unrecognized = r
        .unrecognized
        .iter()
        .map(|u| serde_json::to_value(u).unwrap_or_default())
        .collect();
    let groups = a
        .components
        .groups
        .iter()
        .map(|g| {
            let mut v = serde_json::to_value(g).unwrap_or_default();
            v["first_instance"] =
                serde_json::to_value(node_ref(a, g.instances[0])).unwrap_or_default();
            v
        })
        .collect();
    let variants = a
        .components
        .variants
        .iter()
        .map(|v| serde_json::to_value(v).unwrap_or_default())
        .collect();
    let summary = Summary {
        url: a.doc.url.clone(),
        viewport: a.doc.viewport,
        controls: r.controls,
        recognized_controls: recognized,
        native_links: r.native,
        unrecognized_controls: r.unrecognized.len(),
        by_primitive: r.controls_by_primitive.clone(),
        non_button_primitive_controls: non_button,
        patterns: r.matches.len(),
        js_files_loaded: a.js.len(),
        js_files_with_parse_errors: a
            .js
            .iter()
            .filter(|f| f.parse_errors > 0)
            .map(|f| f.file.clone())
            .collect(),
        js_files_skipped: a
            .js
            .iter()
            .filter_map(|f| Some((f.file.clone(), f.skipped_reason.clone()?)))
            .collect(),
        group_components: emitted.group_components.len(),
        near_duplicate_pairs: a.components.variants.len(),
        near_duplicate_search_truncated: a.components.variants_truncated,
        ambiguous_script_links: r.ambiguous_links,
        repeated_groups: a.components.groups.len(),
        extractable_groups: a.components.groups.iter().filter(|g| g.extractable).count(),
        emitted_components: emitted.components.len() + emitted.group_components.len(),
        skipped_patterns: emitted.skipped.len(),
    };
    Report {
        schema_version: 1,
        page: a.page.clone(),
        summary,
        matches,
        unrecognized,
        groups,
        variants,
        emitted,
        limits: vec![
            "Evidence is the initial captured state of one page and viewport: states the capture never entered (open menus, other tabs) are known only from the recovered scripts, and only where they were recovered.".into(),
            "Script facts come from original React/JSX bundles; sites built without JSX call sites (hand-written DOM scripts) yield DOM/ARIA evidence only.".into(),
            "Links from DOM elements to script sites use shared class tokens, literal props and rendered text; ties are reported as ambiguous and link nothing.".into(),
            "Confidence is a rule score, not a probability. No emitted output has been compared to the original in a browser; parity belongs to the parity pipeline.".into(),
            "Original class names are kept on emitted elements until the styles workstream supplies Tailwind classes; state-modifier classes are dropped and listed.".into(),
        ],
    }
}

pub fn markdown(a: &PageAnalysis, rep: &Report) -> String {
    let s = &rep.summary;
    let mut o = String::new();
    let _ = writeln!(o, "# Semantic report\n");
    let _ = writeln!(
        o,
        "- page: `{}`\n- url: {}\n- viewport: {:?}",
        rep.page, s.url, s.viewport
    );
    let _ = writeln!(
        o,
        "- controls: {} ({} bound to a Base UI primitive, {} of them not `Button`; {} native links; {} unrecognized)",
        s.controls,
        s.recognized_controls,
        s.non_button_primitive_controls,
        s.native_links,
        s.unrecognized_controls
    );
    let _ = writeln!(o, "- by primitive: {:?}", s.by_primitive);
    let _ = writeln!(
        o,
        "- scripts: {} bundle files loaded, {} ambiguous DOM<->script links, parse failures: {:?}, skipped: {:?}",
        s.js_files_loaded,
        s.ambiguous_script_links,
        s.js_files_with_parse_errors,
        s.js_files_skipped
    );
    let _ = writeln!(
        o,
        "- repeated groups: {} ({} extractable with semantic names); emitted components: {}; patterns skipped: {}\n",
        s.repeated_groups, s.extractable_groups, s.emitted_components, s.skipped_patterns
    );
    let _ = writeln!(o, "## Recognized patterns\n");
    let buttons: Vec<String> = a
        .recognition
        .matches
        .iter()
        .filter(|m| m.primitive == Primitive::Button)
        .map(|m| m.root.to_string())
        .collect();
    let _ = writeln!(
        o,
        "Plain `Button` matches ({}; Base UI Button via `components/ui/button.tsx`, nodes): {}\n",
        buttons.len(),
        buttons.join(", ")
    );
    for m in &a.recognition.matches {
        if m.primitive == Primitive::Button {
            continue;
        }
        let root = node_ref(a, m.root);
        let _ = writeln!(
            o,
            "### [{}] {:?} — `{}` (confidence {:.2})\n\nroot: node {} `<{}>`{} {}",
            m.id,
            m.primitive,
            m.rule,
            m.confidence,
            root.id,
            root.tag,
            root.name
                .as_deref()
                .map(|n| format!(" “{n}”"))
                .unwrap_or_default(),
            if root.classes.is_empty() {
                String::new()
            } else {
                format!("(.{})", root.classes.join("."))
            }
        );
        if !m.props.is_empty() {
            let _ = writeln!(o, "props: {:?}", m.props);
        }
        let _ = writeln!(o, "state: {:?}\n", m.state);
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for p in &m.parts {
            *counts.entry(p.part.as_str()).or_default() += 1;
        }
        let _ = writeln!(o, "parts: {:?}", counts);
        for e in &m.evidence {
            let _ = writeln!(
                o,
                "- evidence ({:?}, {:+.2}) {} — {}",
                e.source, e.weight, e.rule, e.detail
            );
        }
        if let Some(j) = &m.js {
            let _ = writeln!(o, "\nrecovered from `{}`:", j.file);
            for d in &j.description {
                let _ = writeln!(o, "- {d}");
            }
            let _ = writeln!(
                o,
                "- provided by the primitive: {}",
                j.provided_by_primitive.join("; ")
            );
            let _ = writeln!(
                o,
                "- stays application code: {}",
                if j.app_code.is_empty() {
                    "none".into()
                } else {
                    j.app_code.join("; ")
                }
            );
        }
        for g in &m.gaps {
            let _ = writeln!(o, "- GAP: {g}");
        }
        let _ = writeln!(o);
    }
    let _ = writeln!(
        o,
        "## Unrecognized controls ({})\n",
        a.recognition.unrecognized.len()
    );
    for u in &a.recognition.unrecognized {
        let _ = writeln!(
            o,
            "- node {} `<{}>` role={:?} name={:?}: {}",
            u.node, u.tag, u.role, u.name, u.reason
        );
    }
    let _ = writeln!(o, "\n## Repeated structures\n");
    for g in &a.components.groups {
        let _ = writeln!(
            o,
            "- {}× subtree of {} elements, first node {}: name {} ({}), {} slot(s){}{}",
            g.instances.len(),
            g.size,
            g.instances[0],
            g.component_name.as_deref().unwrap_or("**unresolved**"),
            g.name_source.as_deref().unwrap_or("no source"),
            g.slots.len(),
            if g.extractable {
                ", extractable"
            } else {
                ", NOT extracted"
            },
            if g.gaps.is_empty() {
                String::new()
            } else {
                format!(" — {}", g.gaps.join("; "))
            }
        );
    }
    let _ = writeln!(
        o,
        "\nNear-duplicate sibling pairs (small tree edit distance, not extracted): {}{}",
        a.components.variants.len(),
        if a.components.variants_truncated {
            " (search truncated at its sibling/pair caps: not every pair was compared)"
        } else {
            ""
        }
    );
    for v in a.components.variants.iter().take(20) {
        let edits: Vec<String> = v
            .edits
            .iter()
            .map(|e| format!("{}{}×{}", e.side, e.count, e.label))
            .collect();
        let _ = writeln!(
            o,
            "- nodes {} and {}: distance {} of {} ({})",
            v.a,
            v.b,
            v.distance,
            v.size,
            edits.join(", ")
        );
    }
    let _ = writeln!(o, "\n## Emitted\n");
    for g in &rep.emitted.group_components {
        let _ = writeln!(
            o,
            "- `{}` ← {} repeated instances of {}; props: {}",
            g.file,
            g.instances.len(),
            g.name,
            g.props.join(", ")
        );
    }
    for g in &rep.emitted.skipped_groups {
        let _ = writeln!(o, "- not emitted: {g}");
    }
    for c in &rep.emitted.components {
        let _ = writeln!(
            o,
            "- `{}` ← match {} ({:?}), {}; name from {}",
            c.file,
            c.match_id,
            c.primitive,
            c.data_file.as_deref().unwrap_or("no data module"),
            c.name_source
        );
        for g in &c.gaps {
            let _ = writeln!(o, "  - GAP: {g}");
        }
    }
    let _ = writeln!(o, "\n## Skipped\n");
    for sk in &rep.emitted.skipped {
        let _ = writeln!(
            o,
            "- match {} ({:?}): {}",
            sk.match_id, sk.primitive, sk.reason
        );
    }
    let st = &rep.emitted;
    let _ = writeln!(
        o,
        "\nDropped while rendering content: {} inline style attributes, state classes {:?}.\n\n## Limits\n",
        st.inline_style_attributes_dropped, st.state_classes_dropped
    );
    for l in &rep.limits {
        let _ = writeln!(o, "- {l}");
    }
    o
}
