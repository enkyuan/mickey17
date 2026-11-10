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

pub use jsx::{
    OriginalClasses, StyleBridge, element_props, is_script_hook, ts_string as ts_string_lit,
};
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

/// The page body with Base UI compositions and repeated components in place.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PageBody {
    /// Children of `<body>` as TSX (indented for a fragment inside `return ( .. )`).
    pub jsx: String,
    /// Hook statements the page component needs (`useState`, `useRouterState`).
    pub hooks: Vec<String>,
    /// Import lines the body and hooks need (components, hooks, types).
    pub imports: Vec<String>,
    /// Components rendered in place, by name.
    pub placed: Vec<String>,
    /// Patterns whose component exists but could not be put in the page, with the reason.
    pub unplaced: Vec<(usize, String)>,
    /// Elements of the body.
    pub elements: usize,
    /// Elements rendered by a composition or a repeated-structure component.
    pub covered_elements: usize,
    /// Elements left as plain markup.
    pub raw_elements: usize,
    /// `script`/`style`/`noscript`/`template` elements not emitted.
    pub scripts_omitted: usize,
    /// Every element deliberately not emitted from the body, by reason.
    pub elements_dropped: std::collections::BTreeMap<String, usize>,
    /// Element nodes rendered by a composition or repeated-structure component.
    #[serde(skip)]
    pub covered_nodes: BTreeSet<crate::semantic::ir::NodeId>,
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
    pub page: Option<PageBody>,
}

pub struct EmitOptions<'a> {
    pub style: &'a dyn StyleBridge,
    /// Directory segment for components and data (`src/components/<ns>/`); empty for none.
    pub namespace: &'a str,
    /// Also render the page body with patterns and repeated components in place.
    pub page: bool,
    /// Maps an asset URL as written in the capture (`src`, `srcset`, `poster`) to the path the
    /// assembled project serves it from; `None` keeps the value.
    pub urls: Option<jsx::UrlMap<'a>>,
}

impl Default for EmitOptions<'_> {
    fn default() -> Self {
        EmitOptions {
            style: &OriginalClasses,
            namespace: "",
            page: false,
            urls: None,
        }
    }
}

/// Emit `components/ui`, compositions and data modules for the recognized patterns.
pub fn emit(analysis: &PageAnalysis, options: &EmitOptions<'_>) -> Emitted {
    patterns::emit(analysis, options)
}
