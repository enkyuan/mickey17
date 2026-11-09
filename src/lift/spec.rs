//! Specification and manifest types of `mochi lift`.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

pub(super) const ALLOWED_GLOBALS: &[&str] = &[
    "Math",
    "Number",
    "Infinity",
    "NaN",
    "undefined",
    "isFinite",
    "isNaN",
    "Array",
    "Map",
];
pub(super) const MATH_MEMBERS: &[&str] = &[
    "abs", "acos", "acosh", "asin", "asinh", "atan", "atan2", "atanh", "cbrt", "ceil", "clz32",
    "cos", "cosh", "exp", "expm1", "floor", "fround", "hypot", "imul", "log", "log10", "log1p",
    "log2", "max", "min", "pow", "round", "sign", "sin", "sinh", "sqrt", "tan", "tanh", "trunc",
    "E", "LN10", "LN2", "LOG10E", "LOG2E", "PI", "SQRT1_2", "SQRT2",
];
pub(super) const NUMBER_MEMBERS: &[&str] = &[
    "isFinite",
    "isInteger",
    "isNaN",
    "isSafeInteger",
    "parseFloat",
    "parseInt",
    "EPSILON",
    "MAX_SAFE_INTEGER",
    "MIN_SAFE_INTEGER",
    "MAX_VALUE",
    "MIN_VALUE",
    "POSITIVE_INFINITY",
    "NEGATIVE_INFINITY",
    "NaN",
];
pub(super) const ARRAY_MEMBERS: &[&str] = &["from", "isArray", "of"];
/// Method names that mutate their receiver; a call on a captured root is a member write.
pub(super) const MUTATORS: &[&str] = &[
    "push",
    "pop",
    "shift",
    "unshift",
    "splice",
    "sort",
    "reverse",
    "fill",
    "copyWithin",
    "set",
    "delete",
    "clear",
    "add",
    "assign",
    "defineProperty",
    "defineProperties",
    "setPrototypeOf",
];
pub(super) const RESERVED: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "let",
    "static",
    "await",
    "async",
    "arguments",
    "eval",
    "implements",
    "interface",
    "package",
    "private",
    "protected",
    "public",
];
pub(super) const MAX_SOURCE: u64 = 16 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub schema_version: u32,
    /// Recovered source, relative to the specification file.
    pub source: PathBuf,
    /// Pins the reviewed names and signatures to exactly this source.
    pub source_sha256: String,
    /// Inclusive 1-based line range whose unlifted functions are reported.
    pub region: Option<Region>,
    /// Verbatim `import type` lines for names used in reviewed signatures.
    #[serde(default)]
    pub imports: Vec<String>,
    /// Verbatim single-line `export type ...;` declarations for names used in
    /// reviewed signatures (for example the value a memoized computation returns).
    #[serde(default)]
    pub declarations: Vec<String>,
    /// Typed bags of explicit inputs that replace captured variables.
    #[serde(default)]
    pub contexts: BTreeMap<String, ContextDef>,
    pub functions: Vec<FunctionSpec>,
}
/// One explicit typed context: every captured value a lifted function may read.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ContextDef {
    pub doc: Option<String>,
    pub fields: Vec<ContextField>,
}
/// A captured read turned into a field. `from` is a captured variable
/// (`z`), a static member path on one (`y.finalCenterY`, `m.config.petalLength`),
/// or a zero-argument call of a captured function whose result is sampled
/// outside the lifted code (`R()`).
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ContextField {
    pub from: String,
    pub name: String,
    pub r#type: String,
    /// Where the value comes from at runtime; recorded for reviewers.
    pub doc: Option<String>,
}
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ContextUse {
    pub r#type: String,
    /// Name of the appended context parameter.
    pub param: String,
}
/// A write-back memo cache the function owns. It is accepted only when the
/// lifter proves recomputation is output-neutral.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MemoSpec {
    /// Captured cache variable (`L`).
    pub cache: String,
    /// Export name of the extracted pure computation that fills the cache.
    pub compute_as: String,
    /// TypeScript type of the cached value.
    pub value_type: String,
    /// Local that holds the computed value in the lifted function.
    pub result_local: String,
    /// Other caches whose invalidation must be coupled to this one.
    #[serde(default)]
    pub coupled: Vec<CoupledCache>,
    /// Inputs whose stability this tool cannot show (for example an object built by an
    /// imported factory). Each is accepted on a stated reason and recorded in the manifest.
    #[serde(default)]
    pub reviewed_assumptions: Vec<ReviewedAssumption>,
}
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ReviewedAssumption {
    /// The captured variable (`m`).
    pub root: String,
    /// Why a reviewer accepts it; appears in the manifest.
    pub reason: String,
    /// The methods the reviewer saw called on it (other than plain read methods). The lift
    /// fails if the source calls different ones.
    #[serde(default)]
    pub methods: Vec<String>,
    /// Member paths below the root (`config.upperHeight`) the reviewer vouches are plain numbers,
    /// strings or booleans, so reading them into a variable or argument is not an alias.
    #[serde(default)]
    pub primitives: Vec<String>,
}
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct CoupledCache {
    /// Captured cache variable inside the sampler (`U`).
    pub cache: String,
    /// The `from` of the context field (`R()`) whose function fills it.
    pub sampler: String,
}
/// A function that only reads another function's cache after calling it.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MemoRead {
    pub cache: String,
    /// Export name of the memoizing function that fills the cache.
    pub via: String,
}
#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub start_line: usize,
    pub end_line: usize,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionSpec {
    /// Minified binding name in the source.
    pub name: String,
    /// 1-based line of that binding, to disambiguate reused minified names.
    pub line: usize,
    /// Exported semantic name.
    #[serde(rename = "as")]
    pub export: String,
    pub doc: Option<String>,
    pub params: Vec<ParamSpec>,
    pub returns: String,
    /// Optional renames for locals declared inside the body, by original name.
    #[serde(default)]
    pub locals: BTreeMap<String, String>,
    /// Explicit typed inputs replacing captured reads.
    pub context: Option<ContextUse>,
    pub memo: Option<MemoSpec>,
    pub memo_read: Option<MemoRead>,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ParamSpec {
    pub from: String,
    pub name: String,
    pub r#type: String,
}

#[derive(Debug, Serialize)]
pub struct Original {
    pub name: String,
    pub line: usize,
    pub kind: &'static str,
    /// Byte range of the function node (declaration, or the initializer of a
    /// `const`/`let` declarator) in the pinned source.
    pub start: u32,
    pub end: u32,
    pub sha256: String,
}
#[derive(Debug, Serialize)]
pub struct LiftedFunction {
    pub export: String,
    pub original: Original,
    pub deps: Vec<String>,
    pub params: Vec<ParamSpec>,
    pub returns: String,
    /// Context type, parameter name and the field names this function reads
    /// directly (dependencies may read more).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<MemoProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo_read: Option<MemoReadRecord>,
}
#[derive(Debug, Serialize)]
pub struct ContextRecord {
    pub r#type: String,
    pub param: String,
    pub reads: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct MemoReadRecord {
    pub cache: String,
    pub via: String,
    pub compute: String,
}
/// What the lifter established about a removed memo cache.
#[derive(Debug, Serialize)]
pub struct MemoProof {
    pub cache: String,
    pub compute: String,
    /// Source of the key expression, which reads only these fields.
    pub key: String,
    pub key_fields: Vec<String>,
    /// Every write to the cache in the whole module.
    pub cache_writes: Vec<WriteSite>,
    /// Inputs not covered by the key, with why each cannot change under the cache.
    pub stable_inputs: Vec<StableInput>,
    pub coupled: Vec<CoupledProof>,
    /// Stability accepted on a reviewer's stated reason, not proven by this tool.
    pub reviewed_assumptions: Vec<String>,
    /// Lines in this module that call the memoizing function. The original
    /// returns one shared cached object; each of these is a place that could
    /// observe the difference if it mutated or compared the result by identity.
    pub callers: Vec<usize>,
    /// What the proof does not cover.
    pub residual: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct WriteSite {
    pub line: usize,
    pub kind: String,
}
#[derive(Debug, Serialize)]
pub struct StableInput {
    pub field: String,
    pub evidence: String,
}
#[derive(Debug, Serialize)]
pub struct CoupledProof {
    pub cache: String,
    pub sampler: String,
    pub fills: Vec<WriteSite>,
    pub resets_paired_with_cache_reset: Vec<usize>,
}
#[derive(Debug, Serialize)]
pub struct Unlifted {
    pub name: String,
    pub line: usize,
    pub reasons: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub tool_version: &'static str,
    pub source: String,
    pub source_sha256: String,
    pub module_sha256: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub contexts: BTreeMap<String, ContextDef>,
    pub functions: Vec<LiftedFunction>,
    /// Functions in the requested region that were not lifted, with reasons.
    pub unlifted: Vec<Unlifted>,
}
