//! Facts recovered from an original React bundle with oxc: JSX call sites with
//! their static props, `useState` state machines, the handlers that move them,
//! and effects that lock scroll, close on Escape or on a media query. Everything
//! is resolved through oxc symbol tables, never by matching minified names or text.
mod extract;

use serde::Serialize;
use std::collections::BTreeMap;

pub use extract::extract;

pub type StateId = usize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Lit {
    Bool(bool),
    Num(f64),
    Str(String),
    Null,
    Undefined,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateVar {
    pub id: StateId,
    /// Binding names in the bundle (minified), kept only as provenance.
    pub name: String,
    pub setter: String,
    pub initial: Option<Lit>,
    /// Source text of a non-literal initializer.
    pub initial_source: Option<String>,
    pub line: usize,
    pub function: usize,
}

/// Where a handler's or binding's key comes from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KeyRef {
    /// The item parameter of the `n`th `.map` callback; `items` when the array is a static literal.
    LoopItem {
        loop_id: usize,
        items: Option<Vec<String>>,
    },
    /// The index parameter of a `.map` callback.
    LoopIndex {
        loop_id: usize,
    },
    /// A literal passed at the call site (`toggle("products")`).
    Literal {
        value: Lit,
    },
    Unknown {
        source: String,
    },
}

/// A summary of an expression, only as precise as the rules need.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ex {
    Lit {
        value: Lit,
    },
    State {
        state: StateId,
    },
    Key {
        key: KeyRef,
    },
    Not {
        arg: Box<Ex>,
    },
    Eq {
        left: Box<Ex>,
        right: Box<Ex>,
    },
    Cond {
        test: Box<Ex>,
        then: Box<Ex>,
        otherwise: Box<Ex>,
    },
    And {
        left: Box<Ex>,
        right: Box<Ex>,
    },
    Other {
        source: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StateEffect {
    Set {
        state: StateId,
        value: Ex,
    },
    /// `set(v => !v)` / `set(!state)`.
    Toggle {
        state: StateId,
    },
    /// `set(key)`.
    SetKey {
        state: StateId,
        key: KeyRef,
    },
    /// `set(v => v === key ? null : key)`.
    ToggleKey {
        state: StateId,
        key: KeyRef,
    },
    Other {
        state: StateId,
        source: String,
    },
}

impl StateEffect {
    pub fn state(&self) -> StateId {
        match self {
            Self::Set { state, .. }
            | Self::Toggle { state }
            | Self::SetKey { state, .. }
            | Self::ToggleKey { state, .. }
            | Self::Other { state, .. } => *state,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Handler {
    pub event: String,
    pub effects: Vec<StateEffect>,
}

/// One `jsx(tag, props)` call.
#[derive(Debug, Clone, Serialize)]
pub struct Site {
    pub id: usize,
    pub line: usize,
    pub tag: String,
    /// `true` when the tag is a string literal (an intrinsic element).
    pub intrinsic: bool,
    /// Whitespace separated class tokens that are present whatever the state.
    pub class_tokens: Vec<String>,
    /// Class tokens added by a state-dependent branch of a template or ternary.
    pub class_conditional: Vec<(String, Ex)>,
    pub static_props: BTreeMap<String, String>,
    pub bindings: Vec<(String, Ex)>,
    /// `...cond && { prop: value }` spreads.
    pub conditional_props: Vec<(Ex, Vec<(String, Ex)>)>,
    pub handlers: Vec<Handler>,
    pub function: usize,
    pub loops: Vec<usize>,
    /// Sites passed as this site's `children` (including through hoisted variables).
    pub children: Vec<usize>,
    pub parent: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EffectFact {
    /// `document.body.style.overflow = <state ? "hidden" : "">`.
    ScrollLock { state: StateId },
    /// A `keydown` listener testing `.key === "Escape"` that runs these effects.
    EscapeKey { effects: Vec<StateEffect> },
    /// `matchMedia(query)` change listener running these effects.
    MediaQuery {
        query: String,
        /// `Some(false)`: the listener acts only once the query stops matching
        /// (`e.matches || close()`); `Some(true)`: only while it matches; `None`: unguarded.
        when_matches: Option<bool>,
        effects: Vec<StateEffect>,
    },
    /// `useEffect(fn, deps)` whose body only moves state: runs `effects` when `deps`
    /// (non-state values, source text) change.
    DepsChange {
        deps: Vec<String>,
        /// `false` when the body only moves state under a condition (`if (x) set(..)`).
        unconditional: bool,
        effects: Vec<StateEffect>,
    },
}

/// An effect-like fact found anywhere in the bundle, with the chain of
/// enclosing functions (innermost last) so it can be tied to a component's state.
#[derive(Debug, Clone, Serialize)]
pub struct EffectRecord {
    pub line: usize,
    pub chain: Vec<usize>,
    pub fact: EffectFact,
}

#[derive(Debug, Clone, Serialize)]
pub struct FnInfo {
    pub id: usize,
    /// Declared name in the bundle (often minified), provenance only.
    pub name: Option<String>,
    /// Keys of a destructured first parameter: real prop names survive minification.
    pub prop_names: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct JsFacts {
    pub file: String,
    pub states: Vec<StateVar>,
    pub sites: Vec<Site>,
    pub effects: Vec<EffectRecord>,
    pub functions: Vec<FnInfo>,
    pub loops: Vec<LoopInfo>,
    pub parse_errors: usize,
    /// Why a script the page loaded was not read (too large, missing from `site/`, unreadable).
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoopInfo {
    pub id: usize,
    pub line: usize,
    pub items: Option<Vec<String>>,
    /// Source of the mapped expression (e.g. a destructured prop), provenance.
    pub source: String,
}

impl JsFacts {
    pub fn state(&self, id: StateId) -> &StateVar {
        &self.states[id]
    }
}
