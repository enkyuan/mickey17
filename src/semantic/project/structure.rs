//! Offline structural comparison of an assembled project with its capture: the elements of the
//! captured page body against the elements of the page the project server-renders, aligned as
//! trees (children matched by tag and own text, recursively), with every element that has no
//! counterpart sorted into a cause. It reads no geometry and runs no browser: it says which
//! elements the generator does not emit, which it adds, and which only moved (wrapped, hoisted),
//! so that the parity pipeline's missing/extra counts can be split into structure and matching.
use crate::semantic::ir::{Doc, NodeId};
use serde::Serialize;
use std::collections::BTreeMap;

/// What the comparison found for one route.
#[derive(Debug, Default, Serialize)]
pub struct RouteStructure {
    pub route: String,
    /// Why the route was not compared (for example no rendered page was supplied).
    pub note: Option<String>,
    /// Elements below `<body>` in the capture / in the server-rendered page.
    pub captured_elements: usize,
    pub rendered_elements: usize,
    /// Elements with a counterpart (same place in the aligned trees, same tag).
    pub matched: usize,
    /// Captured elements with no counterpart, by cause.
    pub missing: BTreeMap<String, usize>,
    /// Rendered elements with no counterpart, by cause.
    pub extra: BTreeMap<String, usize>,
    pub missing_examples: BTreeMap<String, Vec<String>>,
    pub extra_examples: BTreeMap<String, Vec<String>>,
    /// The largest unmatched subtrees (path, elements in the subtree), to see where a gap sits.
    pub missing_roots: Vec<(String, usize)>,
    pub extra_roots: Vec<(String, usize)>,
}

const NEVER_EMITTED: &[&str] = &["script", "style", "noscript", "template", "link", "meta"];
const FRAMEWORK: &[&str] = &["next-route-announcer", "nextjs-portal"];
const EXAMPLES: usize = 4;

fn body_of(doc: &Doc) -> Option<NodeId> {
    doc.nodes
        .iter()
        .find(|n| n.is_element() && n.tag == "body")
        .map(|n| n.id)
}

fn element_children(doc: &Doc, id: NodeId) -> Vec<NodeId> {
    doc.node(id)
        .children
        .iter()
        .copied()
        .filter(|c| doc.node(*c).is_element())
        .collect()
}

/// The text an element holds directly (its own text nodes), whitespace-normalized.
fn own_text(doc: &Doc, id: NodeId) -> String {
    doc.node(id)
        .children
        .iter()
        .filter_map(|c| doc.node(*c).text.as_deref())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Tag plus own text (or the asset it shows): what identifies an element that moved.
fn signature(doc: &Doc, id: NodeId) -> Option<String> {
    let n = doc.node(id);
    let text = own_text(doc, id);
    let target = n.attr("src").or_else(|| n.attr("href")).unwrap_or("");
    if text.is_empty() && target.is_empty() {
        return None;
    }
    Some(format!("{}|{}|{}", n.tag, text, target))
}

fn path_of(doc: &Doc, id: NodeId) -> String {
    let mut parts: Vec<String> = doc
        .ancestors(id)
        .filter(|a| doc.node(*a).is_element())
        .map(|a| doc.node(a).tag.clone())
        .collect();
    parts.reverse();
    parts.push(doc.node(id).tag.clone());
    parts.join(">")
}

fn subtree(doc: &Doc, id: NodeId) -> Vec<NodeId> {
    let mut out = vec![id];
    out.extend(
        doc.descendants(id)
            .into_iter()
            .filter(|d| doc.node(*d).is_element()),
    );
    out
}

/// Weighted longest common subsequence of two child lists: equal tags pair, equal own text pairs
/// better. Returns the index pairs.
fn align(a: &Doc, ac: &[NodeId], b: &Doc, bc: &[NodeId]) -> Vec<(usize, usize)> {
    let (n, m) = (ac.len(), bc.len());
    if n == 0 || m == 0 {
        return Vec::new();
    }
    // Very wide sibling lists: pair by position while tags agree.
    if n * m > 4_000_000 {
        return (0..n.min(m))
            .filter(|&i| a.node(ac[i]).tag == b.node(bc[i]).tag)
            .map(|i| (i, i))
            .collect();
    }
    let sa: Vec<String> = ac.iter().map(|&x| own_text(a, x)).collect();
    let sb: Vec<String> = bc.iter().map(|&x| own_text(b, x)).collect();
    let score = |i: usize, j: usize| -> u32 {
        if a.node(ac[i]).tag != b.node(bc[j]).tag {
            0
        } else if !sa[i].is_empty() && sa[i] == sb[j] {
            3
        } else {
            1
        }
    };
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let take = match score(i, j) {
                0 => 0,
                s => s + dp[i + 1][j + 1],
            };
            dp[i][j] = take.max(dp[i + 1][j]).max(dp[i][j + 1]);
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut pairs = Vec::new();
    while i < n && j < m {
        let s = score(i, j);
        if s > 0 && dp[i][j] == s + dp[i + 1][j + 1] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

struct Walk<'a> {
    a: &'a Doc,
    b: &'a Doc,
    matched: usize,
    missing: Vec<NodeId>,
    extra: Vec<NodeId>,
}

impl Walk<'_> {
    fn visit(&mut self, a: NodeId, b: NodeId) {
        let ac = element_children(self.a, a);
        let bc = element_children(self.b, b);
        let pairs = align(self.a, &ac, self.b, &bc);
        let (mut ia, mut ib) = (vec![false; ac.len()], vec![false; bc.len()]);
        for &(i, j) in &pairs {
            ia[i] = true;
            ib[j] = true;
            self.matched += 1;
            self.visit(ac[i], bc[j]);
        }
        for (i, &c) in ac.iter().enumerate() {
            if !ia[i] {
                self.missing.push(c);
            }
        }
        for (j, &c) in bc.iter().enumerate() {
            if !ib[j] {
                self.extra.push(c);
            }
        }
    }
}

fn bump(
    map: &mut BTreeMap<String, usize>,
    examples: &mut BTreeMap<String, Vec<String>>,
    cause: &str,
    by: usize,
    example: String,
) {
    *map.entry(cause.to_owned()).or_default() += by;
    let e = examples.entry(cause.to_owned()).or_default();
    if e.len() < EXAMPLES {
        e.push(example);
    }
}

/// Compare the captured page (`captured`) with the server-rendered HTML (`rendered`).
pub fn compare(
    route: &str,
    captured: &Doc,
    rendered: &Doc,
    dialog_composed: bool,
) -> RouteStructure {
    let mut out = RouteStructure {
        route: route.to_owned(),
        ..Default::default()
    };
    let (Some(ba), Some(bb)) = (body_of(captured), body_of(rendered)) else {
        return out;
    };
    out.captured_elements = subtree(captured, ba).len() - 1;
    out.rendered_elements = subtree(rendered, bb).len() - 1;
    let mut walk = Walk {
        a: captured,
        b: rendered,
        matched: 0,
        missing: Vec::new(),
        extra: Vec::new(),
    };
    walk.visit(ba, bb);
    out.matched = walk.matched;

    // Elements that exist on both sides but not at the same place: sorted by signature.
    let mut extra_sigs: BTreeMap<String, usize> = BTreeMap::new();
    for &r in &walk.extra {
        for n in subtree(rendered, r) {
            if let Some(s) = signature(rendered, n) {
                *extra_sigs.entry(s).or_default() += 1;
            }
        }
    }
    let mut missing_sigs: BTreeMap<String, usize> = BTreeMap::new();
    for &r in &walk.missing {
        // A subtree that is absent by design is not something that can have moved.
        let tag = captured.node(r).tag.as_str();
        if NEVER_EMITTED.contains(&tag) || FRAMEWORK.contains(&tag) || tag == "iframe" {
            continue;
        }
        for n in subtree(captured, r) {
            if let Some(s) = signature(captured, n) {
                *missing_sigs.entry(s).or_default() += 1;
            }
        }
    }

    let top = |doc: &Doc, roots: &[NodeId]| -> Vec<(String, usize)> {
        let mut v: Vec<(String, usize)> = roots
            .iter()
            .map(|&r| (path_of(doc, r), subtree(doc, r).len()))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v.truncate(8);
        v
    };
    out.missing_roots = top(captured, &walk.missing);
    out.extra_roots = top(rendered, &walk.extra);
    for &root in &walk.missing {
        let root_tag = captured.node(root).tag.clone();
        let root_cause = if NEVER_EMITTED.contains(&root_tag.as_str()) {
            Some("script, style, link, meta or template: not emitted by design")
        } else if FRAMEWORK.contains(&root_tag.as_str()) {
            Some("original framework's runtime element: not emitted by design")
        } else if root_tag == "iframe" {
            Some("iframe")
        } else if dialog_composed && captured.node(root).role() == Some("dialog") {
            Some(
                "dialog popup: Base UI mounts it in a portal on the client, so it is absent from the server HTML",
            )
        } else {
            None
        };
        for n in subtree(captured, root) {
            let node = captured.node(n);
            let cause = match root_cause {
                Some(c) => c,
                None => {
                    if signature(captured, n)
                        .and_then(|s| extra_sigs.get_mut(&s).filter(|c| **c > 0).map(|c| *c -= 1))
                        .is_some()
                    {
                        "moved: the same element is rendered under a different parent (wrapped, hoisted or duplicated)"
                    } else if captured.hidden(n) {
                        "hidden in the capture (display:none, hidden, aria-hidden or visibility:hidden)"
                    } else if node.tag == "svg"
                        || captured.ancestors(n).any(|a| captured.node(a).tag == "svg")
                    {
                        "inside an svg"
                    } else {
                        "no counterpart found"
                    }
                }
            };
            bump(
                &mut out.missing,
                &mut out.missing_examples,
                cause,
                1,
                path_of(captured, n),
            );
        }
    }
    // Rendered-only elements: whatever the missing side did not claim as moved.
    for &root in &walk.extra {
        for n in subtree(rendered, root) {
            let node = rendered.node(n);
            let tag = node.tag.as_str();
            let cause = if NEVER_EMITTED.contains(&tag) || tag == "title" {
                "script, style, link or template rendered by the framework"
            } else if signature(rendered, n)
                .and_then(|s| {
                    missing_sigs
                        .get_mut(&s)
                        .filter(|c| **c > 0)
                        .map(|c| *c -= 1)
                })
                .is_some()
            {
                "moved: the same element is rendered under a different parent (wrapped, hoisted or duplicated)"
            } else if node
                .attrs
                .keys()
                .any(|k| k.starts_with("data-base-ui") || k.starts_with("data-radix"))
                || (tag == "input" && node.attr("aria-hidden") == Some("true"))
                || (matches!(tag, "span" | "div")
                    && node.attr("aria-hidden") == Some("true")
                    && rendered.node(n).children.is_empty())
            {
                "markup a Base UI part adds"
            } else if matches!(tag, "tbody" | "thead" | "tfoot") {
                "table section the HTML parser inserts"
            } else if rendered.ancestors(n).any(|a| rendered.node(a).tag == "svg") {
                "inside an svg"
            } else {
                "no counterpart found"
            };
            bump(
                &mut out.extra,
                &mut out.extra_examples,
                cause,
                1,
                path_of(rendered, n),
            );
        }
    }
    out
}

pub fn markdown(routes: &[RouteStructure]) -> String {
    let mut out = String::from(
        "# Structural comparison (offline)\n\nCaptured page body against the server-rendered page body, children aligned by tag and own text. No geometry, no browser: this separates elements that are not there from elements the parity matcher may fail to pair.\n\n",
    );
    for r in routes {
        if let Some(note) = &r.note {
            out.push_str(&format!("## {}\n\nnot compared: {note}\n\n", r.route));
            continue;
        }
        out.push_str(&format!(
            "## {}\n\n- captured elements {}, rendered {}, matched {}\n",
            r.route, r.captured_elements, r.rendered_elements, r.matched
        ));
        for (title, map, examples) in [
            ("missing in the render", &r.missing, &r.missing_examples),
            ("extra in the render", &r.extra, &r.extra_examples),
        ] {
            out.push_str(&format!("- {title}: {}\n", map.values().sum::<usize>()));
            for (cause, n) in map {
                out.push_str(&format!("  - {n} × {cause}\n"));
                for e in examples.get(cause).into_iter().flatten() {
                    out.push_str(&format!("    - `{e}`\n"));
                }
            }
        }
        for (title, roots) in [
            ("largest missing subtrees", &r.missing_roots),
            ("largest extra subtrees", &r.extra_roots),
        ] {
            if !roots.is_empty() {
                out.push_str(&format!("- {title}:\n"));
                for (path, n) in roots {
                    out.push_str(&format!("  - {n} × `{path}`\n"));
                }
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmatched_elements_get_a_cause_and_moved_elements_are_not_missing() {
        let captured = Doc::from_html(
            "<html><body><div><p>a</p><p>b</p></div><script>x</script><ul><li>one</li></ul><span hidden>h</span><b hidden>z</b></body></html>",
        );
        let rendered = Doc::from_html(
            "<html><body><div><p>a</p></div><section><p>b</p></section><ul><li>one</li></ul><span hidden>h</span><i data-base-ui-x>q</i></body></html>",
        );
        let r = compare("/", &captured, &rendered, true);
        assert_eq!(r.captured_elements, 8);
        assert!(
            r.missing
                .keys()
                .any(|k| k.contains("not emitted by design")),
            "{:?}",
            r.missing
        );
        assert!(
            r.missing.keys().any(|k| k.starts_with("moved")),
            "{:?}",
            r.missing
        );
        assert!(
            r.missing.keys().any(|k| k.starts_with("hidden")),
            "{:?}",
            r.missing
        );
        assert!(
            r.extra.keys().any(|k| k.starts_with("moved")),
            "{:?}",
            r.extra
        );
        assert!(
            r.extra.keys().any(|k| k.contains("Base UI")),
            "{:?}",
            r.extra
        );
        // Every element is accounted for exactly once.
        assert_eq!(
            r.matched + r.missing.values().sum::<usize>(),
            r.captured_elements
        );
        assert_eq!(
            r.matched + r.extra.values().sum::<usize>(),
            r.rendered_elements
        );
    }
}
