//! Which element of one viewport's snapshot is which element of another's.
//!
//! The captures of one page at several viewport widths share most of their DOM, but scripts add,
//! remove and re-class elements per width (`home-engines--compact` at 375, not at 1440). Elements
//! are paired only when their tag, identity attributes and *normalized* classes (modifier
//! suffixes and state-like classes stripped) are equal and they sit in the same place of the
//! aligned parent: a wrong pair would give an element another element's styles, and the check
//! compares against the paired element, so it could not notice.

use super::super::capture::Snapshot;
use super::super::selector::Dom;
use std::collections::HashMap;

/// A class that says what the element is, not what state or variant it is in.
fn stable_class(c: &str) -> Option<String> {
    let base = c.split("--").next().unwrap_or(c);
    let lower = base.to_ascii_lowercase();
    if base.is_empty()
        || lower.starts_with("is-")
        || lower.starts_with("has-")
        || lower.starts_with("js-")
        || matches!(
            lower.as_str(),
            "active" | "open" | "closed" | "visible" | "hidden" | "show" | "hide" | "loaded"
        )
    {
        return None;
    }
    Some(base.to_string())
}

/// Equality key of an element: what must match for two elements to be the same one.
pub fn key(snap: &Snapshot, node: usize) -> String {
    let mut classes: Vec<String> = snap
        .attr(node, "class")
        .unwrap_or("")
        .split_ascii_whitespace()
        .filter_map(stable_class)
        .collect();
    classes.sort();
    classes.dedup();
    let attr = |n: &str| snap.attr(node, n).unwrap_or("");
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        snap.tag(node),
        classes.join("."),
        attr("id"),
        attr("role"),
        attr("href"),
        attr("src"),
        attr("type"),
        attr("name"),
        attr("for"),
        attr("alt"),
    )
}

/// Pair the elements of `b` with those of `a` (`a` index -> `b` index). Children of paired parents
/// are matched by the longest common subsequence of their keys.
pub fn align(a: &Snapshot, b: &Snapshot) -> HashMap<usize, usize> {
    let mut out = HashMap::new();
    let root = |s: &Snapshot| s.elements().find(|&n| s.parent(n).is_none());
    let (Some(ra), Some(rb)) = (root(a), root(b)) else {
        return out;
    };
    if key(a, ra) != key(b, rb) {
        return out;
    }
    let mut stack = vec![(ra, rb)];
    while let Some((x, y)) = stack.pop() {
        out.insert(x, y);
        let (ca, cb) = (a.element_children(x), b.element_children(y));
        let (ka, kb): (Vec<String>, Vec<String>) = (
            ca.iter().map(|&n| key(a, n)).collect(),
            cb.iter().map(|&n| key(b, n)).collect(),
        );
        for (i, j) in lcs(&ka, &kb) {
            // Equal siblings that one capture has more of than the other (a carousel slide
            // removed, a list item added) cannot be told apart by order: leave them unpaired.
            let n = |list: &[String], k: &str| list.iter().filter(|x| *x == k).count();
            if n(&ka, &ka[i]) == n(&kb, &kb[j]) {
                stack.push((ca[i], cb[j]));
            }
        }
    }
    out
}

/// Longest common subsequence of two key lists, as index pairs.
fn lcs(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 {
        return Vec::new();
    }
    // Siblings beyond this many make the quadratic table not worth it: pair by position only
    // while the keys agree.
    if n * m > 4_000_000 {
        return (0..n.min(m))
            .filter(|&i| a[i] == b[i])
            .map(|i| (i, i))
            .collect();
    }
    let mut table = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if a[i] == b[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|i| (*i).to_string()).collect()
    }

    #[test]
    fn common_subsequence_pairs_equal_keys_in_order() {
        let pairs = lcs(&s(&["a", "b", "c", "d"]), &s(&["x", "a", "c", "d", "y"]));
        assert_eq!(pairs, vec![(0, 1), (2, 2), (3, 3)]);
        assert!(lcs(&s(&[]), &s(&["a"])).is_empty());
    }

    #[test]
    fn modifier_and_state_classes_do_not_make_elements_different() {
        assert_eq!(stable_class("card--compact").as_deref(), Some("card"));
        assert_eq!(stable_class("is-open"), None);
        assert_eq!(stable_class("js-toggle"), None);
        assert_eq!(stable_class("hero__title").as_deref(), Some("hero__title"));
    }
}
