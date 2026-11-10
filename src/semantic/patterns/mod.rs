//! Pattern recognition: a rule table over the accessibility tree, computed
//! style and (when the original bundle is available) recovered state machines
//! maps captured controls to Base UI primitives. Every match carries its
//! evidence, a confidence and its gaps; every interactive element no rule
//! accepts is reported as unrecognized with the reason.
mod bind;
mod link;
mod matcher;
mod rules;

use super::ir::NodeId;
use serde::Serialize;
use std::collections::BTreeMap;

pub use bind::slug;
pub use link::{Link, Links};
pub use matcher::recognize;
pub use rules::{Candidate, Check, CheckKind, Pred, RULES, RootSel, Rule};

/// The Base UI primitive a pattern maps to (`@base-ui/react/<module>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Primitive {
    Accordion,
    AlertDialog,
    Button,
    Checkbox,
    Collapsible,
    Combobox,
    Dialog,
    Field,
    Menu,
    Menubar,
    NavigationMenu,
    Popover,
    RadioGroup,
    Select,
    Switch,
    Tabs,
    Toggle,
    ToggleGroup,
    Tooltip,
}

impl Primitive {
    /// The import specifier (`@base-ui/react/tabs`).
    pub fn module(self) -> &'static str {
        match self {
            Self::Accordion => "@base-ui/react/accordion",
            Self::AlertDialog => "@base-ui/react/alert-dialog",
            Self::Button => "@base-ui/react/button",
            Self::Checkbox => "@base-ui/react/checkbox",
            Self::Collapsible => "@base-ui/react/collapsible",
            Self::Combobox => "@base-ui/react/combobox",
            Self::Dialog => "@base-ui/react/dialog",
            Self::Field => "@base-ui/react/field",
            Self::Menu => "@base-ui/react/menu",
            Self::Menubar => "@base-ui/react/menubar",
            Self::NavigationMenu => "@base-ui/react/navigation-menu",
            Self::Popover => "@base-ui/react/popover",
            Self::RadioGroup => "@base-ui/react/radio-group",
            Self::Select => "@base-ui/react/select",
            Self::Switch => "@base-ui/react/switch",
            Self::Tabs => "@base-ui/react/tabs",
            Self::Toggle => "@base-ui/react/toggle",
            Self::ToggleGroup => "@base-ui/react/toggle-group",
            Self::Tooltip => "@base-ui/react/tooltip",
        }
    }
    /// File name of the thin wrapper under `components/ui/`.
    pub fn ui_file(self) -> &'static str {
        match self {
            Self::AlertDialog => "alert-dialog",
            Self::NavigationMenu => "navigation-menu",
            Self::RadioGroup => "radio-group",
            Self::ToggleGroup => "toggle-group",
            other => other.module().rsplit('/').next().unwrap(),
        }
    }
    /// Exported parts of the primitive in Base UI 1.9, used to reject a binder that
    /// names a part that does not exist.
    pub fn parts(self) -> &'static [&'static str] {
        match self {
            Self::Accordion => &["Root", "Item", "Header", "Trigger", "Panel"],
            Self::AlertDialog => &[
                "Root",
                "Trigger",
                "Portal",
                "Backdrop",
                "Viewport",
                "Popup",
                "Title",
                "Description",
                "Close",
            ],
            Self::Button => &[],
            Self::Checkbox => &["Root", "Indicator"],
            Self::Collapsible => &["Root", "Trigger", "Panel"],
            Self::Combobox => &[
                "Root",
                "Label",
                "Value",
                "Input",
                "InputGroup",
                "Trigger",
                "List",
                "Status",
                "Portal",
                "Backdrop",
                "Positioner",
                "Popup",
                "Arrow",
                "Icon",
                "Group",
                "GroupLabel",
                "Item",
                "ItemIndicator",
                "Chips",
                "Chip",
                "ChipRemove",
                "Row",
                "Collection",
                "Empty",
                "Clear",
                "Separator",
            ],
            Self::Dialog => &[
                "Root",
                "Trigger",
                "Portal",
                "Backdrop",
                "Viewport",
                "Popup",
                "Title",
                "Description",
                "Close",
            ],
            Self::Field => &[
                "Root",
                "Label",
                "Error",
                "Description",
                "Control",
                "Validity",
                "Item",
            ],
            Self::Menu => &[
                "Root",
                "Trigger",
                "Portal",
                "Backdrop",
                "Positioner",
                "Popup",
                "Arrow",
                "Viewport",
                "Item",
                "LinkItem",
                "CheckboxItem",
                "CheckboxItemIndicator",
                "RadioGroup",
                "RadioItem",
                "RadioItemIndicator",
                "Group",
                "GroupLabel",
                "SubmenuRoot",
                "SubmenuTrigger",
            ],
            Self::Menubar => &[],
            Self::NavigationMenu => &[
                "Root",
                "List",
                "Item",
                "Trigger",
                "Content",
                "Link",
                "Icon",
                "Portal",
                "Positioner",
                "Popup",
                "Viewport",
                "Backdrop",
                "Arrow",
            ],
            Self::Popover => &[
                "Root",
                "Trigger",
                "Portal",
                "Positioner",
                "Popup",
                "Arrow",
                "Backdrop",
                "Title",
                "Description",
                "Close",
                "Viewport",
            ],
            Self::RadioGroup => &[],
            Self::Select => &[
                "Root",
                "Label",
                "Trigger",
                "Value",
                "Icon",
                "Portal",
                "Backdrop",
                "Positioner",
                "Popup",
                "List",
                "Item",
                "ItemIndicator",
                "ItemText",
                "Arrow",
                "ScrollDownArrow",
                "ScrollUpArrow",
                "Group",
                "GroupLabel",
                "Separator",
            ],
            Self::Switch => &["Root", "Thumb"],
            Self::Tabs => &["Root", "List", "Tab", "Indicator", "Panel"],
            Self::Toggle => &[],
            Self::ToggleGroup => &[],
            Self::Tooltip => &[
                "Root",
                "Trigger",
                "Portal",
                "Positioner",
                "Popup",
                "Arrow",
                "Provider",
                "Viewport",
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    Dom,
    Aria,
    Style,
    Js,
    Structure,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    pub rule: String,
    pub source: EvidenceSource,
    pub detail: String,
    pub node: Option<NodeId>,
    /// Weight the check contributed (positive supports, negative contradicts).
    pub weight: f32,
}

/// A Base UI part bound to a captured node.
#[derive(Debug, Clone, Serialize)]
pub struct Part {
    pub part: String,
    pub node: NodeId,
    /// Stable item key (tab value, accordion value) and where it came from.
    pub key: Option<String>,
    pub key_source: Option<String>,
    /// Element the part renders as when it is not the primitive's default
    /// (`a` for a link tab, `li` for an accordion item, `h3` for a header).
    pub render_as: Option<String>,
}

/// How the state of the pattern is owned.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum StateModel {
    /// Base UI owns the state; `default` is the value observed in the capture.
    Uncontrolled {
        default: Option<String>,
        observed_from: String,
    },
    /// The original keeps the state in a component; others move it too.
    Controlled {
        state: String,
        /// The initial value as the bundle writes it (a literal); `None` when it is a prop or
        /// an expression.
        initial: Option<String>,
        /// What the capture showed (the value at the captured instant).
        observed: Option<String>,
        moved_by_others: Vec<String>,
    },
    /// No state at all (a static pattern).
    Static,
}

/// Original-bundle facts attached to a match.
#[derive(Debug, Clone, Serialize)]
pub struct JsBinding {
    pub file: String,
    pub state: Option<usize>,
    pub description: Vec<String>,
    /// Behaviors the original implemented by hand that the primitive provides.
    pub provided_by_primitive: Vec<String>,
    /// Behaviors the original has that the primitive does not: kept as app code.
    pub app_code: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Match {
    pub id: usize,
    pub primitive: Primitive,
    pub rule: String,
    pub root: NodeId,
    pub label: Option<String>,
    pub parts: Vec<Part>,
    pub props: BTreeMap<String, String>,
    pub state: StateModel,
    pub confidence: f32,
    pub evidence: Vec<Evidence>,
    pub gaps: Vec<String>,
    pub js: Option<JsBinding>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Unrecognized {
    pub node: NodeId,
    pub tag: String,
    pub role: Option<String>,
    pub name: Option<String>,
    pub reason: String,
    /// Rules that matched part of their evidence: `(rule, why it was rejected)`.
    pub near_misses: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Recognition {
    pub matches: Vec<Match>,
    pub unrecognized: Vec<Unrecognized>,
    /// Interactive elements (buttons, links, inputs, role-based controls) in the page.
    pub controls: usize,
    /// Controls that belong to a recognized match, by primitive.
    pub controls_by_primitive: BTreeMap<Primitive, usize>,
    /// Native links and other controls with no Base UI counterpart.
    pub native: usize,
    pub ambiguous_links: usize,
}
