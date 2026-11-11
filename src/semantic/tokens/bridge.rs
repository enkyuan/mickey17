//! `emit::StyleBridge` over a style translation: the emitted JSX gets the utilities (or the
//! named recipe) the cascade analysis produced for each captured element.

use super::styles::run::StylePlan;
use crate::semantic::emit::StyleBridge;
use crate::semantic::ir::{Doc, NodeId};
use std::cell::RefCell;
use std::collections::BTreeSet;

pub struct TailwindBridge<'a> {
    plan: &'a StylePlan,
    /// Elements asked for that the translation did not cover (they keep their original classes).
    pub missed: RefCell<BTreeSet<NodeId>>,
}

impl StylePlan {
    pub fn bridge(&self) -> TailwindBridge<'_> {
        TailwindBridge {
            plan: self,
            missed: RefCell::new(BTreeSet::new()),
        }
    }
}

impl StyleBridge for TailwindBridge<'_> {
    /// The translation reads inline declarations into the cascade (their winners become classes).
    fn translates_inline_styles(&self) -> bool {
        true
    }

    fn class_name(&self, _doc: &Doc, node: NodeId, original: &[String]) -> String {
        match self.plan.classes.get(&node) {
            Some(classes) => classes.clone(),
            None => {
                self.missed.borrow_mut().insert(node);
                original.join(" ")
            }
        }
    }
}
