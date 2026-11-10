//! The semantic IR: the captured DOM with its accessibility facts resolved
//! (effective role, accessible name, ARIA state) and the computed style facts
//! the pattern rules need. Every node keeps the index it had in the capture, so
//! each later decision can cite its evidence by node id.
mod aria;
mod build;

use serde::Serialize;
use std::collections::BTreeMap;

pub use aria::{implicit_role, is_interactive_role};
pub use build::STYLE_KEYS;

pub type NodeId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleSource {
    Explicit,
    Implicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NameSource {
    LabelledBy,
    AriaLabel,
    NativeLabel,
    Content,
    Title,
    Placeholder,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    /// Lowercase tag for elements (SVG keeps its camelCase); empty for text.
    pub tag: String,
    /// Text nodes only.
    pub text: Option<String>,
    pub attrs: BTreeMap<String, String>,
    pub classes: Vec<String>,
    /// Selected computed properties ([`STYLE_KEYS`]); empty for HTML sources.
    pub style: BTreeMap<String, String>,
    pub bounds: Option<Rect>,
    pub role: Option<String>,
    pub role_source: Option<RoleSource>,
    pub name: Option<String>,
    pub name_source: Option<NameSource>,
}

impl Node {
    pub fn is_element(&self) -> bool {
        self.text.is_none()
    }
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }
    pub fn has_attr(&self, name: &str) -> bool {
        self.attrs.contains_key(name)
    }
    pub fn has_class(&self, class: &str) -> bool {
        self.classes.iter().any(|c| c == class)
    }
    pub fn role(&self) -> Option<&str> {
        self.role.as_deref()
    }
    /// `true`/`false` for a literal ARIA boolean; `None` when absent or `mixed`/`undefined`.
    pub fn aria_bool(&self, name: &str) -> Option<bool> {
        match self.attr(name)?.trim().to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }
    pub fn style(&self, key: &str) -> Option<&str> {
        self.style.get(key).map(String::as_str)
    }
    /// Space separated id references (`aria-controls`, `aria-labelledby`, ...).
    pub fn id_refs(&self, name: &str) -> Vec<&str> {
        self.attr(name)
            .map(|v| v.split_whitespace().collect())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Doc {
    pub url: String,
    pub title: String,
    pub viewport: Option<(u32, u32)>,
    pub nodes: Vec<Node>,
    /// First element carrying each `id` attribute.
    pub by_id: BTreeMap<String, NodeId>,
}

impl Doc {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }
    pub fn element_children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes[id]
            .children
            .iter()
            .copied()
            .filter(|c| self.nodes[*c].is_element())
    }
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.nodes[id].parent, |p| self.nodes[*p].parent)
    }
    /// Pre-order descendants, excluding `id` itself.
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.nodes[id].children.iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            out.push(n);
            stack.extend(self.nodes[n].children.iter().rev().copied());
        }
        out
    }
    pub fn is_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        self.ancestors(node).any(|a| a == ancestor)
    }
    pub fn lowest_common_ancestor(&self, ids: &[NodeId]) -> Option<NodeId> {
        let first = *ids.first()?;
        let mut chain: Vec<NodeId> = std::iter::once(first)
            .chain(self.ancestors(first))
            .collect();
        for &other in &ids[1..] {
            chain.retain(|c| *c == other || self.is_ancestor(*c, other));
        }
        chain.first().copied()
    }
    pub fn resolve(&self, id_ref: &str) -> Option<NodeId> {
        self.by_id.get(id_ref).copied()
    }
    /// Controlled elements from `aria-controls`, in order, ignoring dangling ids.
    pub fn controlled(&self, id: NodeId) -> Vec<NodeId> {
        self.nodes[id]
            .id_refs("aria-controls")
            .into_iter()
            .filter_map(|r| self.resolve(r))
            .collect()
    }
    /// The element whose `aria-controls` or `aria-labelledby` names `id`.
    pub fn controllers_of(&self, id: NodeId) -> Vec<NodeId> {
        let Some(dom_id) = self.nodes[id].attr("id") else {
            return Vec::new();
        };
        self.nodes
            .iter()
            .filter(|n| {
                n.is_element()
                    && (n.id_refs("aria-controls").contains(&dom_id)
                        || n.id_refs("aria-owns").contains(&dom_id))
            })
            .map(|n| n.id)
            .collect()
    }
    /// Removed from the rendering or accessibility tree by the node itself.
    pub fn self_hidden(&self, id: NodeId) -> bool {
        let n = &self.nodes[id];
        n.aria_bool("aria-hidden") == Some(true)
            || n.has_attr("hidden")
            || n.style("display") == Some("none")
            || n.style("visibility") == Some("hidden")
    }
    pub fn hidden(&self, id: NodeId) -> bool {
        self.self_hidden(id) || self.ancestors(id).any(|a| self.self_hidden(a))
    }
    pub fn inert(&self, id: NodeId) -> bool {
        let inert = |n: NodeId| {
            let node = &self.nodes[n];
            node.has_attr("inert") && node.attr("inert") != Some("false")
        };
        inert(id) || self.ancestors(id).any(inert)
    }
    /// Whitespace-normalized rendered text of the subtree (skips hidden subtrees,
    /// `script` and `style`), cut at `limit` characters.
    pub fn text_content(&self, id: NodeId, limit: usize) -> String {
        let mut out = String::new();
        self.collect_text(id, limit, &mut out, 0, true);
        let joined = out.split_whitespace().collect::<Vec<_>>().join(" ");
        joined.chars().take(limit).collect()
    }
    /// Like [`Doc::text_content`] but images contribute nothing: only text that is on the page.
    pub fn visible_text(&self, id: NodeId, limit: usize) -> String {
        let mut out = String::new();
        self.collect_text(id, limit, &mut out, 0, false);
        let joined = out.split_whitespace().collect::<Vec<_>>().join(" ");
        joined.chars().take(limit).collect()
    }
    fn collect_text(&self, id: NodeId, limit: usize, out: &mut String, depth: usize, alt: bool) {
        if out.len() > limit * 4 || depth > 512 {
            return;
        }
        let n = &self.nodes[id];
        if let Some(t) = &n.text {
            out.push_str(t);
            return;
        }
        // Content hidden from assistive technology or not rendered at all is not text;
        // `visibility`/opacity are animation states of the captured instant and do not count.
        let excluded = n.aria_bool("aria-hidden") == Some(true)
            || n.has_attr("hidden")
            || n.style("display") == Some("none");
        if matches!(n.tag.as_str(), "script" | "style" | "template") || excluded {
            return;
        }
        if n.tag == "img" {
            if let Some(alt) = n.attr("alt").filter(|_| alt) {
                out.push(' ');
                out.push_str(alt);
                out.push(' ');
            }
            return;
        }
        let block = !matches!(
            n.tag.as_str(),
            "span" | "a" | "b" | "i" | "em" | "strong" | "code" | "small" | "sup" | "sub"
        );
        if block {
            out.push(' ');
        }
        for &c in &n.children {
            self.collect_text(c, limit, out, depth + 1, alt);
        }
        if block {
            out.push(' ');
        }
    }
    /// Visible interactive descendants: native and role-based controls.
    pub fn interactive_descendants(&self, id: NodeId) -> Vec<NodeId> {
        self.descendants(id)
            .into_iter()
            .filter(|d| {
                let n = &self.nodes[*d];
                n.is_element()
                    && n.role().map(aria::is_interactive_role).unwrap_or(false)
                    && !self.hidden(*d)
            })
            .collect()
    }
}
