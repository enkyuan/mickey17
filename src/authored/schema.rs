use crate::project::SourceRef;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A reviewable semantic program, not a DOM replay or a container for source strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Specification {
    pub schema_version: u32,
    pub input_sha256: String,
    pub routes: Vec<Route>,
    pub components: BTreeMap<String, Component>,
    #[serde(default)]
    pub source_modules: BTreeMap<String, SourceModule>,
    #[serde(default)]
    pub source_assets: BTreeMap<String, SourceAsset>,
    #[serde(default)]
    pub external_components: BTreeMap<String, ExternalComponent>,
    #[serde(default)]
    pub libraries: Vec<String>,
    #[serde(default)]
    pub assets: BTreeMap<String, String>,
    pub tokens: BTreeMap<String, String>,
    #[serde(default)]
    pub token_scopes: Vec<TokenScope>,
    #[serde(default)]
    pub fonts: Vec<Font>,
    #[serde(default)]
    pub globals: Vec<Rule>,
    /// Observable behavior still requiring reconstruction. Never silently cleared by emission.
    #[serde(default)]
    pub gaps: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub path: String,
    pub title: String,
    pub component: String,
    #[serde(default)]
    pub data_name: Option<String>,
    pub props: BTreeMap<String, serde_json::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// ui and layout select shared components; other names select a feature.
    pub feature: String,
    #[serde(default)]
    pub group: Option<String>,
    pub purpose: String,
    pub evidence: Vec<SourceRef>,
    #[serde(default)]
    pub props: BTreeMap<String, Type>,
    #[serde(default)]
    pub state: BTreeMap<String, State>,
    pub body: Node,
    #[serde(default)]
    pub styles: Vec<Rule>,
    #[serde(default)]
    pub behaviors: Vec<Behavior>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Type {
    Text,
    Number,
    Boolean,
    Href,
    Asset,
    List { item: Box<Type> },
    Object { fields: BTreeMap<String, Type> },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub value_type: Type,
    pub initial: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Expression {
    Assign {
        state: String,
        value: Box<Expression>,
    },
    Object {
        fields: BTreeMap<String, Expression>,
    },
    List {
        items: Vec<Expression>,
    },
    /// A shared persisted accessibility choice; no recovered global controller.
    Preference {
        value: Preference,
    },
    Literal {
        value: serde_json::Value,
    },
    Ref {
        path: String,
    },
    Asset {
        name: String,
    },
    Setter {
        state: String,
    },
    /// Explicit local behavior, e.g. disclosure or a fixture submission. Never invokes an API.
    Set {
        state: String,
        value: serde_json::Value,
        #[serde(default)]
        prevent_default: bool,
    },
    /// Element-form Base UI render composition; no manual prop/event merging.
    Link {
        href: Box<Expression>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    External {
        name: String,
        #[serde(default)]
        props: BTreeMap<String, Expression>,
    },
    If {
        condition: Expression,
        then: Box<Node>,
        otherwise: Box<Node>,
    },
    Text {
        value: Expression,
    },
    Element {
        tag: String,
        #[serde(default)]
        props: BTreeMap<String, Expression>,
        #[serde(default)]
        classes: Vec<String>,
        #[serde(default)]
        children: Vec<Node>,
    },
    Primitive {
        /// Direct Base UI API, e.g. Dialog.Popup or NavigationMenu.Trigger.
        name: String,
        #[serde(default)]
        props: BTreeMap<String, Expression>,
        #[serde(default)]
        classes: Vec<String>,
        #[serde(default)]
        children: Vec<Node>,
    },
    Component {
        name: String,
        #[serde(default)]
        props: BTreeMap<String, Expression>,
    },
    Repeat {
        each: Expression,
        binding: String,
        key: Expression,
        body: Box<Node>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub selector: String,
    pub declarations: BTreeMap<String, String>,
    #[serde(default)]
    pub media: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenScope {
    pub selector: String,
    #[serde(default)]
    pub media: Option<String>,
    pub values: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Font {
    pub family: String,
    pub asset: String,
    pub weight: String,
    pub style: String,
    pub display: String,
    #[serde(default)]
    pub unicode_range: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Behavior {
    ScrollHeader {
        minimum_width: u32,
    },
    Reveal {
        class: String,
        effect: RevealEffect,
        distance: f64,
        duration: u32,
        stagger: u32,
        threshold: f64,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevealEffect {
    Fade,
    Rise,
    Scale,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preference {
    AccessibilityMode,
    ReducedMotion,
    SetAccessibilityMode,
}

/// Owned application code for custom algorithms. Files are relative to the spec,
/// copied as normal source, and hashed/rechecked. No recovered runtime is accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceModule {
    pub file: String,
    pub purpose: String,
    pub evidence: Vec<SourceRef>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalComponent {
    pub module: String,
    #[serde(default)]
    pub props: BTreeMap<String, Type>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAsset {
    pub file: String,
    pub sha256: String,
    pub purpose: String,
    pub evidence: Vec<SourceRef>,
}
