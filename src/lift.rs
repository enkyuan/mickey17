//! Provenance-linked lifting of closed functions out of recovered controllers.
//!
//! A function is liftable only when every identifier it uses is a parameter, a
//! local, a standard global from a small allowlist, or another liftable
//! function. Anything else (captured state, `this`, writes to outer variables)
//! blocks the lift and is reported with its reason; nothing is guessed.
//! Semantic names and TypeScript signatures come from a reviewed specification
//! that is pinned to the source by SHA-256. Bodies are the original code with
//! identifiers renamed through resolved scope information.
use crate::cache::fingerprint;
use anyhow::{Context, Result, bail, ensure};
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_semantic::{ScopeFlags, Scoping, SemanticBuilder};
use oxc_span::{GetSpan, SourceType, Span};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
};

const ALLOWED_GLOBALS: &[&str] = &[
    "Math",
    "Number",
    "Infinity",
    "NaN",
    "undefined",
    "isFinite",
    "isNaN",
    "Array",
];
const RESERVED: &[&str] = &[
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
];
const MAX_SOURCE: u64 = 16 * 1024 * 1024;

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
    pub functions: Vec<FunctionSpec>,
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
    pub functions: Vec<LiftedFunction>,
    /// Functions in the requested region that were not lifted, with reasons.
    pub unlifted: Vec<Unlifted>,
}

struct Candidate {
    name: String,
    line: usize,
    symbol: oxc_semantic::SymbolId,
    node: Span,
    kind: &'static str,
    params: Vec<(String, oxc_semantic::SymbolId)>,
    body: Span,
    expression: bool,
    blockers: BTreeSet<String>,
    deps: BTreeSet<usize>,
    closed: bool,
}
struct Ref {
    span: Span,
    symbol: Option<oxc_semantic::SymbolId>,
    name: String,
    write: bool,
}
struct Bind {
    span: Span,
    symbol: oxc_semantic::SymbolId,
    name: String,
}
struct Collector<'s> {
    scoping: &'s Scoping,
    lines: &'s [usize],
    candidates: Vec<Candidate>,
    refs: Vec<Ref>,
    binds: Vec<Bind>,
    this_spans: Vec<Span>,
    shorthand: Vec<Span>,
}
fn line_of(lines: &[usize], offset: u32) -> usize {
    lines.partition_point(|start| *start <= offset as usize)
}
struct Shape {
    node: Span,
    kind: &'static str,
    body: Span,
    expression: bool,
    /// `async` or generator, which this pass does not lift.
    modifiers: bool,
}
impl Collector<'_> {
    fn function_like(
        &mut self,
        id: &BindingIdentifier<'_>,
        params: &FormalParameters<'_>,
        shape: Shape,
    ) {
        let Shape {
            node,
            kind,
            body,
            expression,
            modifiers,
        } = shape;
        let Some(symbol) = id.symbol_id.get() else {
            return;
        };
        let mut blockers = BTreeSet::new();
        let mut names = Vec::new();
        if modifiers {
            blockers.insert("async or generator function".to_owned());
        }
        if params.rest.is_some() {
            blockers.insert("rest parameter".to_owned());
        }
        for item in &params.items {
            match (&item.pattern, &item.initializer) {
                (BindingPattern::BindingIdentifier(p), None) => {
                    if let Some(s) = p.symbol_id.get() {
                        names.push((p.name.to_string(), s));
                    }
                }
                _ => {
                    blockers.insert("destructured or defaulted parameter".to_owned());
                }
            }
        }
        self.candidates.push(Candidate {
            name: id.name.to_string(),
            line: line_of(self.lines, id.span.start),
            symbol,
            node,
            kind,
            params: names,
            body,
            expression,
            blockers,
            deps: BTreeSet::new(),
            closed: false,
        });
    }
}
impl<'a> Visit<'a> for Collector<'_> {
    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        let reference = id.reference_id.get().map(|r| self.scoping.get_reference(r));
        self.refs.push(Ref {
            span: id.span,
            symbol: reference.and_then(|r| r.symbol_id()),
            name: id.name.to_string(),
            write: reference.is_some_and(|r| r.is_write()),
        });
    }
    fn visit_binding_identifier(&mut self, id: &BindingIdentifier<'a>) {
        if let Some(symbol) = id.symbol_id.get() {
            self.binds.push(Bind {
                span: id.span,
                symbol,
                name: id.name.to_string(),
            });
        }
    }
    fn visit_this_expression(&mut self, it: &ThisExpression) {
        self.this_spans.push(it.span);
    }
    fn visit_object_property(&mut self, p: &ObjectProperty<'a>) {
        if p.shorthand {
            self.shorthand.push(p.span);
        }
        walk::walk_object_property(self, p);
    }
    fn visit_binding_property(&mut self, p: &BindingProperty<'a>) {
        if p.shorthand {
            self.shorthand.push(p.span);
        }
        walk::walk_binding_property(self, p);
    }
    fn visit_function(&mut self, f: &Function<'a>, flags: ScopeFlags) {
        if f.r#type == FunctionType::FunctionDeclaration
            && let (Some(id), Some(body)) = (&f.id, &f.body)
        {
            self.function_like(
                id,
                &f.params,
                Shape {
                    node: f.span,
                    kind: "function-declaration",
                    body: body.span,
                    expression: false,
                    modifiers: f.r#async || f.generator,
                },
            );
        }
        walk::walk_function(self, f, flags);
    }
    fn visit_variable_declarator(&mut self, d: &VariableDeclarator<'a>) {
        if let (BindingPattern::BindingIdentifier(id), Some(init)) = (&d.id, &d.init) {
            match init {
                Expression::ArrowFunctionExpression(f) => {
                    let (body, expression) = match &f.body {
                        ArrowFunctionBody::FunctionBody(b) => (b.span, false),
                        other => (other.span(), true),
                    };
                    self.function_like(
                        id,
                        &f.params,
                        Shape {
                            node: f.span,
                            kind: "arrow-declarator",
                            body,
                            expression,
                            modifiers: f.r#async,
                        },
                    );
                }
                Expression::FunctionExpression(f) => {
                    if let Some(body) = &f.body {
                        self.function_like(
                            id,
                            &f.params,
                            Shape {
                                node: f.span,
                                kind: "function-expression-declarator",
                                body: body.span,
                                expression: false,
                                modifiers: f.r#async || f.generator,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
        walk::walk_variable_declarator(self, d);
    }
}
fn within(inner: Span, outer: Span) -> bool {
    inner.start >= outer.start && inner.end <= outer.end
}

struct Analysis {
    candidates: Vec<Candidate>,
    refs: Vec<Ref>,
    binds: Vec<Bind>,
    shorthand: Vec<Span>,
    scoping_symbol_spans: BTreeMap<oxc_semantic::SymbolId, Span>,
}
fn analyze(source: &str) -> Result<Analysis> {
    let arena = Allocator::default();
    let parsed = Parser::new(&arena, source, SourceType::mjs()).parse();
    ensure!(
        parsed.diagnostics.is_empty(),
        "source does not parse as a JavaScript module"
    );
    let sema = SemanticBuilder::new_compiler().build(&parsed.program);
    ensure!(
        sema.diagnostics.is_empty(),
        "source has invalid bindings for semantic analysis"
    );
    let scoping = sema.semantic.scoping();
    let mut lines = vec![0usize];
    lines.extend(
        source
            .bytes()
            .enumerate()
            .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
    );
    let mut collector = Collector {
        scoping,
        lines: &lines,
        candidates: Vec::new(),
        refs: Vec::new(),
        binds: Vec::new(),
        this_spans: Vec::new(),
        shorthand: Vec::new(),
    };
    collector.visit_program(&parsed.program);
    let Collector {
        mut candidates,
        refs,
        binds,
        this_spans,
        shorthand,
        ..
    } = collector;
    let by_symbol: BTreeMap<_, _> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (c.symbol, i))
        .collect();
    let mut spans = BTreeMap::new();
    for candidate in &mut candidates {
        for r in refs.iter().filter(|r| within(r.span, candidate.node)) {
            match r.symbol {
                None if ALLOWED_GLOBALS.contains(&r.name.as_str()) => {}
                None => {
                    candidate.blockers.insert(format!("global `{}`", r.name));
                }
                Some(symbol) if symbol == candidate.symbol => {}
                Some(symbol) => {
                    let declared = *spans
                        .entry(symbol)
                        .or_insert_with(|| scoping.symbol_span(symbol));
                    if within(declared, candidate.node) {
                    } else if let Some(&dep) = by_symbol.get(&symbol) {
                        candidate.deps.insert(dep);
                    } else {
                        candidate.blockers.insert(format!(
                            "{} captured `{}`",
                            if r.write { "writes" } else { "reads" },
                            r.name
                        ));
                    }
                }
            }
        }
        if this_spans.iter().any(|s| within(*s, candidate.node)) {
            candidate.blockers.insert("uses `this`".to_owned());
        }
        candidate.closed = candidate.blockers.is_empty();
    }
    loop {
        let mut changed = false;
        for i in 0..candidates.len() {
            if !candidates[i].closed {
                continue;
            }
            let broken: Vec<String> = candidates[i]
                .deps
                .iter()
                .filter(|d| !candidates[**d].closed)
                .map(|d| format!("depends on `{}`", candidates[*d].name))
                .collect();
            if !broken.is_empty() {
                candidates[i].closed = false;
                candidates[i].blockers.extend(broken);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for candidate in &candidates {
        spans
            .entry(candidate.symbol)
            .or_insert_with(|| scoping.symbol_span(candidate.symbol));
    }
    Ok(Analysis {
        candidates,
        refs,
        binds,
        shorthand,
        scoping_symbol_spans: spans,
    })
}

fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).with_context(|| format!("read {}", path.display()))?;
    ensure!(
        meta.is_file() && meta.len() <= MAX_SOURCE,
        "{} is not a regular file within the byte budget",
        path.display()
    );
    Ok(std::fs::read(path)?)
}
fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !RESERVED.contains(&name)
}
fn valid_type(text: &str) -> bool {
    !text.trim().is_empty() && !text.contains(['\n', ';', '`'])
}

/// Counts leading whitespace characters.
fn indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == '\t' || *c == ' ').count()
}
fn convert(line: &str, remove: usize) -> String {
    let rest = &line[remove.min(line.len())..];
    let width = indent(rest);
    let tabs = rest[..width].chars().filter(|c| *c == '\t').count();
    format!(
        "{}{}",
        "  ".repeat(tabs) + &" ".repeat(width - tabs),
        &rest[width..]
    )
}
fn reindent_block(inner: &str) -> String {
    let lines: Vec<&str> = inner.lines().collect();
    let min = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent(l))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .skip_while(|l| l.trim().is_empty())
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                format!("  {}", convert(l, min))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}
fn reindent_expression(expression: &str) -> String {
    let mut lines = expression.trim().lines();
    let first = lines.next().unwrap_or("").trim();
    let rest: Vec<&str> = lines.collect();
    let min = rest
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent(l))
        .min()
        .unwrap_or(0);
    let mut out = format!("  return {first}");
    for line in rest {
        out.push('\n');
        if !line.trim().is_empty() {
            out.push_str(&format!("  {}", convert(line, min)));
        }
    }
    out.push(';');
    out
}

struct Plan<'a> {
    candidate: usize,
    spec: &'a FunctionSpec,
}

/// Lifts the functions named by `spec_path`, writing a TypeScript module and a
/// provenance manifest. Both output paths must not already exist.
pub fn lift(spec_path: &Path, module_out: &Path, manifest_out: &Path) -> Result<Manifest> {
    let spec: Spec =
        serde_json::from_slice(&read_bounded(spec_path)?).context("parse lift specification")?;
    ensure!(
        spec.schema_version == 1,
        "unsupported lift specification version"
    );
    ensure!(
        !spec.functions.is_empty(),
        "lift specification names no functions"
    );
    let source_path = spec_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(&spec.source);
    let bytes = read_bounded(&source_path)?;
    let digest = fingerprint(&bytes);
    ensure!(
        digest == spec.source_sha256,
        "source hash {digest} does not match the reviewed specification"
    );
    let source = std::str::from_utf8(&bytes).context("source is not UTF-8")?;
    let analysis = analyze(source)?;
    let mut plans = Vec::new();
    let mut seen = BTreeSet::new();
    for function in &spec.functions {
        ensure!(
            valid_identifier(&function.export),
            "`{}` is not a usable export name",
            function.export
        );
        ensure!(
            seen.insert(function.export.clone()),
            "duplicate export `{}`",
            function.export
        );
        ensure!(
            valid_type(&function.returns),
            "return type of `{}` is empty or unsafe",
            function.export
        );
        let found: Vec<usize> = analysis
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.name == function.name && c.line == function.line)
            .map(|(i, _)| i)
            .collect();
        ensure!(
            found.len() == 1,
            "`{}` at line {} matches {} functions",
            function.name,
            function.line,
            found.len()
        );
        let candidate = &analysis.candidates[found[0]];
        ensure!(
            candidate.closed,
            "`{}` at line {} cannot be lifted: {}",
            function.name,
            function.line,
            candidate
                .blockers
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        );
        ensure!(
            candidate.params.len() == function.params.len()
                && candidate
                    .params
                    .iter()
                    .zip(&function.params)
                    .all(|((original, _), reviewed)| *original == reviewed.from),
            "parameters of `{}` at line {} do not match the specification",
            function.name,
            function.line
        );
        for param in &function.params {
            ensure!(
                valid_identifier(&param.name),
                "`{}` is not a usable parameter name",
                param.name
            );
            ensure!(
                valid_type(&param.r#type),
                "type of parameter `{}` is empty or unsafe",
                param.name
            );
        }
        plans.push(Plan {
            candidate: found[0],
            spec: function,
        });
    }
    let exports: BTreeMap<usize, &str> = plans
        .iter()
        .map(|p| (p.candidate, p.spec.export.as_str()))
        .collect();
    let mut module = format!(
        "// Lifted by `mochi lift` from {} (sha256 {}).\n// Bodies keep the original computation; names and signatures come from the\n// reviewed lift specification. Per-function provenance is in {}.\n",
        source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("source"),
        &digest[..12],
        manifest_out
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("the manifest"),
    );
    for import in &spec.imports {
        ensure!(
            import.starts_with("import type ") && import.ends_with(';') && !import.contains('\n'),
            "imports must be single-line `import type ...;` statements"
        );
        module.push_str(import);
        module.push('\n');
    }
    let mut lifted = Vec::new();
    for plan in &plans {
        let candidate = &analysis.candidates[plan.candidate];
        let mut renames: BTreeMap<oxc_semantic::SymbolId, String> = BTreeMap::new();
        for ((_, symbol), reviewed) in candidate.params.iter().zip(&plan.spec.params) {
            renames.insert(*symbol, reviewed.name.clone());
        }
        let mut deps = Vec::new();
        for dep in &candidate.deps {
            let Some(name) = exports.get(dep) else {
                bail!(
                    "`{}` at line {} depends on `{}` at line {}, which the specification must also name",
                    candidate.name,
                    candidate.line,
                    analysis.candidates[*dep].name,
                    analysis.candidates[*dep].line
                );
            };
            renames.insert(analysis.candidates[*dep].symbol, (*name).to_owned());
            deps.push((*name).to_owned());
        }
        renames.insert(candidate.symbol, plan.spec.export.clone());
        let body = candidate.body;
        let mut used_locals = BTreeSet::new();
        // `name#k` renames the k-th (1-based, source order) binding of `name`
        // in the body; a bare `name` renames every binding of that name.
        let mut occurrences: BTreeMap<&str, usize> = BTreeMap::new();
        let mut declared = BTreeSet::new();
        for bind in analysis.binds.iter().filter(|b| within(b.span, body)) {
            let count = occurrences.entry(bind.name.as_str()).or_default();
            *count += 1;
            if !declared.insert(bind.symbol) {
                continue;
            }
            let ordinal = format!("{}#{count}", bind.name);
            let key = [&ordinal, &bind.name]
                .into_iter()
                .find(|k| plan.spec.locals.contains_key(*k));
            if let Some(key) = key
                && !renames.contains_key(&bind.symbol)
            {
                let new = &plan.spec.locals[key];
                ensure!(valid_identifier(new), "`{new}` is not a usable local name");
                renames.insert(bind.symbol, new.clone());
                used_locals.insert(key.clone());
            }
        }
        for name in plan.spec.locals.keys() {
            ensure!(
                used_locals.contains(name),
                "local `{name}` is not declared in `{}`",
                plan.spec.export
            );
        }
        let in_scope = |span: Span| within(span, candidate.node);
        let mut kept = BTreeSet::new();
        for r in analysis.refs.iter().filter(|r| in_scope(r.span)) {
            if !r.symbol.is_some_and(|s| renames.contains_key(&s)) {
                kept.insert(r.name.clone());
            }
        }
        for b in analysis.binds.iter().filter(|b| in_scope(b.span)) {
            if !renames.contains_key(&b.symbol) {
                kept.insert(b.name.clone());
            }
        }
        let mut targets: BTreeMap<&str, String> = BTreeMap::new();
        for (symbol, new) in &renames {
            let old = analysis
                .refs
                .iter()
                .find(|r| r.symbol == Some(*symbol))
                .map(|r| r.name.clone())
                .or_else(|| {
                    analysis
                        .binds
                        .iter()
                        .find(|b| b.symbol == *symbol)
                        .map(|b| b.name.clone())
                })
                .unwrap_or_default();
            ensure!(
                !kept.contains(new),
                "new name `{new}` collides with an identifier kept in `{}`",
                plan.spec.export
            );
            if let Some(previous) = targets.insert(new.as_str(), old.clone()) {
                ensure!(
                    previous == old,
                    "`{new}` would merge distinct identifiers in `{}`",
                    plan.spec.export
                );
            }
        }
        let mut edits: Vec<(usize, usize, &str)> = Vec::new();
        for r in analysis.refs.iter().filter(|r| within(r.span, body)) {
            if let Some(new) = r.symbol.and_then(|s| renames.get(&s)) {
                edits.push((r.span.start as usize, r.span.end as usize, new));
            }
        }
        for b in analysis.binds.iter().filter(|b| within(b.span, body)) {
            if let Some(new) = renames.get(&b.symbol) {
                edits.push((b.span.start as usize, b.span.end as usize, new));
            }
        }
        for (start, end, _) in &edits {
            ensure!(
                !analysis
                    .shorthand
                    .iter()
                    .any(|s| (*start as u32) >= s.start && (*end as u32) <= s.end),
                "renaming inside a shorthand property in `{}` is not supported",
                plan.spec.export
            );
        }
        edits.sort_by_key(|e| e.0);
        let mut text = String::new();
        let mut cursor = body.start as usize;
        for (start, end, new) in edits {
            ensure!(
                start >= cursor,
                "overlapping edits in `{}`",
                plan.spec.export
            );
            text.push_str(&source[cursor..start]);
            text.push_str(new);
            cursor = end;
        }
        text.push_str(&source[cursor..body.end as usize]);
        let rendered = if candidate.expression {
            reindent_expression(&text)
        } else {
            ensure!(
                text.starts_with('{') && text.ends_with('}'),
                "unexpected body shape in `{}`",
                plan.spec.export
            );
            reindent_block(&text[1..text.len() - 1])
        };
        let node_text = &source[candidate.node.start as usize..candidate.node.end as usize];
        let node_hash = fingerprint(node_text.as_bytes());
        let params = plan
            .spec
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name, p.r#type.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        module.push_str(&format!(
            "\n/** {}Lifted from `{}` (line {}, bytes {}..{}, sha256 {}). */\nexport function {}({}): {} {{\n{}\n}}\n",
            plan.spec.doc.as_deref().map(|d| format!("{d} ")).unwrap_or_default(),
            candidate.name,
            candidate.line,
            candidate.node.start,
            candidate.node.end,
            &node_hash[..12],
            plan.spec.export,
            params,
            plan.spec.returns.trim(),
            rendered
        ));
        lifted.push(LiftedFunction {
            export: plan.spec.export.clone(),
            original: Original {
                name: candidate.name.clone(),
                line: candidate.line,
                kind: candidate.kind,
                start: candidate.node.start,
                end: candidate.node.end,
                sha256: node_hash,
            },
            deps,
            params: plan.spec.params.clone(),
            returns: plan.spec.returns.trim().to_owned(),
        });
    }
    let selected: BTreeSet<usize> = plans.iter().map(|p| p.candidate).collect();
    let unlifted = analysis
        .candidates
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            !selected.contains(i)
                && spec
                    .region
                    .is_some_and(|r| c.line >= r.start_line && c.line <= r.end_line)
        })
        .map(|(_, c)| Unlifted {
            name: c.name.clone(),
            line: c.line,
            reasons: if c.closed {
                vec!["closed but not named in the specification".to_owned()]
            } else {
                c.blockers.iter().cloned().collect()
            },
        })
        .collect();
    let _ = &analysis.scoping_symbol_spans;
    let manifest = Manifest {
        schema_version: 1,
        tool_version: env!("CARGO_PKG_VERSION"),
        source: spec.source.to_string_lossy().into_owned(),
        source_sha256: digest,
        module_sha256: fingerprint(module.as_bytes()),
        functions: lifted,
        unlifted,
    };
    write_new(module_out, module.as_bytes())?;
    write_new(manifest_out, &serde_json::to_vec_pretty(&manifest)?)?;
    Ok(manifest)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {} (it must not already exist)", path.display()))?;
    file.write_all(bytes)?;
    Ok(())
}

/// One row per function-like binding in the optional line range.
#[derive(Debug, Serialize)]
pub struct SurveyRow {
    pub name: String,
    pub line: usize,
    pub liftable: bool,
    pub reasons: Vec<String>,
}
pub fn survey(source: &Path, region: Option<Region>) -> Result<Vec<SurveyRow>> {
    let bytes = read_bounded(source)?;
    let analysis = analyze(std::str::from_utf8(&bytes).context("source is not UTF-8")?)?;
    Ok(analysis
        .candidates
        .iter()
        .filter(|c| region.is_none_or(|r| c.line >= r.start_line && c.line <= r.end_line))
        .map(|c| SurveyRow {
            name: c.name.clone(),
            line: c.line,
            liftable: c.closed,
            reasons: c.blockers.iter().cloned().collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "const clamp = (e) => Math.max(0, Math.min(1, e));\nlet z = 0;\nconst lerp = (a, b, t) => a + (b - a) * t;\nfunction ease(e) {\n\tlet t = clamp(e);\n\treturn lerp(0, 1, t * t);\n}\nconst bad = (e) => e + z;\nconst writes = () => { z = 1; };\nconst usesThis = function () { return this; };\n";

    fn write(dir: &Path, spec: &str) -> PathBuf {
        std::fs::write(dir.join("source.js"), SOURCE).unwrap();
        let spec = spec.replace("HASH", &fingerprint(SOURCE.as_bytes()));
        let path = dir.join("lift.json");
        std::fs::write(&path, spec).unwrap();
        path
    }
    fn by_name<'a>(rows: &'a [SurveyRow], name: &str) -> &'a SurveyRow {
        rows.iter().find(|r| r.name == name).unwrap()
    }

    #[test]
    fn survey_separates_closed_functions_from_captured_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("source.js"), SOURCE).unwrap();
        let rows = survey(&dir.path().join("source.js"), None).unwrap();
        for name in ["clamp", "lerp", "ease"] {
            assert!(by_name(&rows, name).liftable, "{name}");
        }
        assert_eq!(by_name(&rows, "bad").reasons, ["reads captured `z`"]);
        assert_eq!(by_name(&rows, "writes").reasons, ["writes captured `z`"]);
        assert_eq!(by_name(&rows, "usesThis").reasons, ["uses `this`"]);
    }

    #[test]
    fn lifts_named_typed_functions_with_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let spec = write(
            dir.path(),
            r#"{"schema_version":1,"source":"source.js","source_sha256":"HASH","functions":[
              {"name":"clamp","line":1,"as":"clamp01","params":[{"from":"e","name":"value","type":"number"}],"returns":"number"},
              {"name":"lerp","line":3,"as":"mix","params":[{"from":"a","name":"start","type":"number"},{"from":"b","name":"end","type":"number"},{"from":"t","name":"amount","type":"number"}],"returns":"number"},
              {"name":"ease","line":4,"as":"easeIn","locals":{"t":"clamped"},"params":[{"from":"e","name":"input","type":"number"}],"returns":"number"}]}"#,
        );
        let out = dir.path().join("math.ts");
        let manifest = lift(&spec, &out, &dir.path().join("math.lift.json")).unwrap();
        let module = std::fs::read_to_string(&out).unwrap();
        assert!(module.contains("export function clamp01(value: number): number {\n  return Math.max(0, Math.min(1, value));\n}"), "{module}");
        assert!(module.contains("export function mix(start: number, end: number, amount: number): number {\n  return start + (end - start) * amount;"), "{module}");
        assert!(
            module
                .contains("let clamped = clamp01(input);\n  return mix(0, 1, clamped * clamped);"),
            "{module}"
        );
        assert_eq!(manifest.functions[2].deps, ["clamp01", "mix"]);
        assert_eq!(manifest.functions[0].original.line, 1);
        assert!(std::fs::metadata(dir.path().join("math.lift.json")).is_ok());
    }

    #[test]
    fn refuses_blocked_stale_and_unnamed_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.ts");
        let manifest = dir.path().join("out.json");
        let blocked = write(
            dir.path(),
            r#"{"schema_version":1,"source":"source.js","source_sha256":"HASH","functions":[
              {"name":"bad","line":8,"as":"bad","params":[{"from":"e","name":"e","type":"number"}],"returns":"number"}]}"#,
        );
        let error = lift(&blocked, &out, &manifest).unwrap_err().to_string();
        assert!(error.contains("reads captured `z`"), "{error}");
        let unnamed = write(
            dir.path(),
            r#"{"schema_version":1,"source":"source.js","source_sha256":"HASH","functions":[
              {"name":"ease","line":4,"as":"easeIn","params":[{"from":"e","name":"input","type":"number"}],"returns":"number"}]}"#,
        );
        let error = lift(&unnamed, &out, &manifest).unwrap_err().to_string();
        assert!(error.contains("must also name"), "{error}");
        let stale = write(
            dir.path(),
            r#"{"schema_version":1,"source":"source.js","source_sha256":"deadbeef","functions":[
              {"name":"clamp","line":1,"as":"clamp01","params":[{"from":"e","name":"value","type":"number"}],"returns":"number"}]}"#,
        );
        let error = lift(&stale, &out, &manifest).unwrap_err().to_string();
        assert!(
            error.contains("does not match the reviewed specification"),
            "{error}"
        );
        assert!(
            !out.exists(),
            "no output may be written when a lift is refused"
        );
    }

    #[test]
    fn rename_collisions_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let merged = write(
            dir.path(),
            r#"{"schema_version":1,"source":"source.js","source_sha256":"HASH","functions":[
              {"name":"lerp","line":3,"as":"mix","params":[{"from":"a","name":"x","type":"number"},{"from":"b","name":"x","type":"number"},{"from":"t","name":"amount","type":"number"}],"returns":"number"}]}"#,
        );
        let error = lift(
            &merged,
            &dir.path().join("o.ts"),
            &dir.path().join("o.json"),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("would merge"), "{error}");
        let shadowing_global = write(
            dir.path(),
            r#"{"schema_version":1,"source":"source.js","source_sha256":"HASH","functions":[
              {"name":"clamp","line":1,"as":"clamp01","params":[{"from":"e","name":"Math","type":"number"}],"returns":"number"}]}"#,
        );
        let error = lift(
            &shadowing_global,
            &dir.path().join("o.ts"),
            &dir.path().join("o.json"),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("collides"), "{error}");
        assert!(!dir.path().join("o.ts").exists());
    }

    #[test]
    fn committed_fixture_is_reproduced_exactly() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("benchmarks/fixtures/lift");
        let out = tempfile::tempdir().unwrap();
        lift(
            &fixture.join("lift.json"),
            &out.path().join("math.ts"),
            &out.path().join("math.lift.json"),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(out.path().join("math.ts")).unwrap(),
            std::fs::read_to_string(fixture.join("math.ts")).unwrap(),
            "regenerate the golden fixture only after reviewing the diff"
        );
        let strip = |path: &Path| {
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            value.as_object_mut().unwrap().remove("tool_version");
            value
        };
        assert_eq!(
            strip(&out.path().join("math.lift.json")),
            strip(&fixture.join("math.lift.json"))
        );
    }

    #[test]
    fn simultaneous_renames_that_swap_names_are_valid() {
        let dir = tempfile::tempdir().unwrap();
        let spec = write(
            dir.path(),
            r#"{"schema_version":1,"source":"source.js","source_sha256":"HASH","functions":[
              {"name":"lerp","line":3,"as":"mix","params":[{"from":"a","name":"b","type":"number"},{"from":"b","name":"a","type":"number"},{"from":"t","name":"amount","type":"number"}],"returns":"number"}]}"#,
        );
        let out = dir.path().join("o.ts");
        lift(&spec, &out, &dir.path().join("o.json")).unwrap();
        assert!(
            std::fs::read_to_string(out)
                .unwrap()
                .contains("return b + (a - b) * amount;")
        );
    }
}
