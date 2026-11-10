//! Ordered tree edit distance (Zhang & Shasha, 1989) between two DOM subtrees,
//! unit insert/delete/relabel costs. A node's label is its tag, role and
//! non-state class blocks; text runs are one `#text` leaf.
use super::{is_state_class, significant_children};
use crate::semantic::ir::{Doc, NodeId};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct TreeEdit {
    /// `a` or `b`: the side that has the label more often than the other.
    pub side: char,
    pub label: String,
    pub count: usize,
}

struct Postorder {
    labels: Vec<String>,
    /// Leftmost leaf descendant of each node (postorder index).
    leftmost: Vec<usize>,
}

fn label(doc: &Doc, id: NodeId) -> String {
    let n = doc.node(id);
    if n.text.is_some() {
        return "#text".into();
    }
    let mut classes: Vec<&str> = n
        .classes
        .iter()
        .map(String::as_str)
        .filter(|c| !is_state_class(c))
        .collect();
    classes.sort_unstable();
    format!("{}[{}]{}", n.tag, n.role().unwrap_or(""), classes.join("."))
}

fn build(doc: &Doc, root: NodeId) -> Postorder {
    fn go(doc: &Doc, id: NodeId, out: &mut Postorder, depth: usize) -> usize {
        // Deeper than any realistic component: treated as a leaf so recursion stays bounded.
        let kids = if depth > 256 {
            Vec::new()
        } else {
            super::collapse_text(doc, significant_children(doc, id))
        };
        let mut leftmost = None;
        for k in kids {
            let l = go(doc, k, out, depth + 1);
            leftmost.get_or_insert(l);
        }
        let index = out.labels.len();
        out.labels.push(label(doc, id));
        let l = leftmost.unwrap_or(index);
        out.leftmost.push(l);
        l
    }
    let mut out = Postorder {
        labels: Vec::new(),
        leftmost: Vec::new(),
    };
    go(doc, root, &mut out, 0);
    out
}

fn keyroots(leftmost: &[usize]) -> Vec<usize> {
    let mut seen: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, l) in leftmost.iter().enumerate() {
        seen.insert(*l, i);
    }
    let mut roots: Vec<usize> = seen.into_values().collect();
    roots.sort_unstable();
    roots
}

/// Exact distance, plus the label multiset difference between the trees (labels
/// present on one side only). The difference explains the distance; it is not an
/// edit script.
pub fn tree_edit_distance(doc: &Doc, a: NodeId, b: NodeId) -> (usize, Vec<TreeEdit>) {
    let t1 = build(doc, a);
    let t2 = build(doc, b);
    let (n1, n2) = (t1.labels.len(), t2.labels.len());
    let mut td = vec![vec![0usize; n2]; n1];
    for &i in &keyroots(&t1.leftmost) {
        for &j in &keyroots(&t2.leftmost) {
            let m = i - t1.leftmost[i] + 2;
            let n = j - t2.leftmost[j] + 2;
            let mut fd = vec![vec![0usize; n]; m];
            for di in 1..m {
                fd[di][0] = fd[di - 1][0] + 1;
            }
            for dj in 1..n {
                fd[0][dj] = fd[0][dj - 1] + 1;
            }
            for di in 1..m {
                for dj in 1..n {
                    let i1 = t1.leftmost[i] + di - 1;
                    let j1 = t2.leftmost[j] + dj - 1;
                    if t1.leftmost[i1] == t1.leftmost[i] && t2.leftmost[j1] == t2.leftmost[j] {
                        let cost = usize::from(t1.labels[i1] != t2.labels[j1]);
                        fd[di][dj] = (fd[di - 1][dj] + 1)
                            .min(fd[di][dj - 1] + 1)
                            .min(fd[di - 1][dj - 1] + cost);
                        td[i1][j1] = fd[di][dj];
                    } else {
                        let p = t1.leftmost[i1] - t1.leftmost[i];
                        let q = t2.leftmost[j1] - t2.leftmost[j];
                        fd[di][dj] = (fd[di - 1][dj] + 1)
                            .min(fd[di][dj - 1] + 1)
                            .min(fd[p][q] + td[i1][j1]);
                    }
                }
            }
        }
    }
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for l in &t1.labels {
        *counts.entry(l).or_default() += 1;
    }
    for l in &t2.labels {
        *counts.entry(l).or_default() -= 1;
    }
    let edits = counts
        .into_iter()
        .filter(|(_, c)| *c != 0)
        .map(|(l, c)| TreeEdit {
            side: if c > 0 { 'a' } else { 'b' },
            label: l.to_owned(),
            count: c.unsigned_abs() as usize,
        })
        .collect();
    (td[n1 - 1][n2 - 1], edits)
}
