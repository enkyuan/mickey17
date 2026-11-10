//! TSX emitters. Direct Base UI primitives appear only in `src/components/ui/`;
//! compositions import those wrappers, carry typed props and data modules, and
//! keep behavior the original implemented by hand and Base UI does not provide
//! (route-change close, media-query close, accordion reset) as explicit app code.
mod jsx;
mod naming;
mod patterns;
mod ts;
mod ui;

use super::PageAnalysis;
use super::patterns::Primitive;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub use jsx::{OriginalClasses, StyleBridge};
pub use ui::wrapper_name;

#[derive(Debug, Clone, Serialize)]
pub struct EmittedComponent {
    pub match_id: usize,
    pub primitive: Primitive,
    pub name: String,
    pub name_source: String,
    pub file: String,
    pub data_file: Option<String>,
    /// Other matches rendered by the same component (identical repeated usage).
    pub also_matches: Vec<usize>,
    pub gaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupComponent {
    pub name: String,
    pub file: String,
    pub instances: Vec<crate::semantic::ir::NodeId>,
    pub props: Vec<String>,
    pub name_source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Skipped {
    pub match_id: usize,
    pub primitive: Primitive,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Emitted {
    /// Project-relative path -> file contents.
    pub files: BTreeMap<String, String>,
    pub components: Vec<EmittedComponent>,
    pub skipped: Vec<Skipped>,
    pub group_components: Vec<GroupComponent>,
    /// Extractable repeated structures not emitted, with the reason (e.g. a name already taken).
    pub skipped_groups: Vec<String>,
    pub ui_primitives: BTreeSet<Primitive>,
    pub inline_style_attributes_dropped: usize,
    pub state_classes_dropped: BTreeSet<String>,
    pub svg_subtrees: usize,
    pub custom_elements: BTreeSet<String>,
    pub custom_attributes: usize,
}

pub struct EmitOptions<'a> {
    pub style: &'a dyn StyleBridge,
}

impl Default for EmitOptions<'_> {
    fn default() -> Self {
        EmitOptions {
            style: &OriginalClasses,
        }
    }
}

/// Emit `components/ui`, compositions and data modules for the recognized patterns.
pub fn emit(analysis: &PageAnalysis, options: &EmitOptions<'_>) -> Emitted {
    patterns::emit(analysis, options)
}
