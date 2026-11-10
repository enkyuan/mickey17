//! `emit::StyleBridge` over a style translation: the emitted JSX gets the utilities (or the
//! named recipe) the cascade analysis produced for each captured element.

use super::styles::run::StylePlan;
use crate::semantic::emit::StyleBridge;
use crate::semantic::ir::{Doc, NodeId};
use std::cell::Cell;

pub struct TailwindBridge<'a> {
    plan: &'a StylePlan,
    /// Elements asked for that the translation did not cover (they keep their original classes).
    pub missed: Cell<usize>,
}

impl StylePlan {
    pub fn bridge(&self) -> TailwindBridge<'_> {
        TailwindBridge {
            plan: self,
            missed: Cell::new(0),
        }
    }
}

impl StyleBridge for TailwindBridge<'_> {
    fn class_name(&self, _doc: &Doc, node: NodeId, original: &[String]) -> String {
        match self.plan.classes.get(&node) {
            Some(classes) => classes.clone(),
            None => {
                self.missed.set(self.missed.get() + 1);
                original.join(" ")
            }
        }
    }
}
