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
use oxc_ast_visit::Visit;
use oxc_span::Span;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

mod analysis;
mod emit;
mod memo;
mod resolve;
mod spec;

use analysis::*;
use emit::*;
use memo::*;
use resolve::*;
pub use spec::*;

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
    let ref_by_start: BTreeMap<u32, usize> = analysis
        .refs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.span.start, i))
        .collect();
    let write_roots: BTreeSet<u32> = analysis.member_writes.iter().map(|w| w.start).collect();
    for r in &analysis.refs {
        let global = r.symbol.is_none()
            && (ALLOWED_GLOBALS.contains(&r.name.as_str()) || r.name == "Object");
        let through = write_roots.contains(&r.span.start)
            && !(r.name == "Object" && analysis.mutator_roots.contains(&r.span.start));
        ensure!(
            !(global && (r.write || through)),
            "the module writes to the standard global `{}` at line {}; lifted code would not see that change",
            r.name,
            line_of(&analysis.lines, r.span.start)
        );
    }
    // Code the static checks cannot see through: direct `eval`/`Function` anywhere in the module,
    // and (for memo proofs) ways to reach the prototype chain or patch `Object.freeze`.
    {
        let uses_context = spec.functions.iter().any(|f| f.context.is_some());
        let uses_memo = spec.functions.iter().any(|f| f.memo.is_some());
        for (at, eval_like, why) in &analysis.hazards {
            ensure!(
                !((*eval_like && uses_context) || uses_memo),
                "{why} (line {}), which can change what the static proofs assume",
                line_of(&analysis.lines, *at)
            );
        }
    }
    let mut context_fields: BTreeMap<&str, Vec<FieldDef<'_>>> = BTreeMap::new();
    for (name, def) in &spec.contexts {
        ensure!(
            valid_identifier(name),
            "`{name}` is not a usable context type name"
        );
        context_fields.insert(name, parse_fields(name, def)?);
    }
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for import in &spec.imports {
        let Some(names) = import_names(import) else {
            bail!("imports must be single-line `import type ...;` statements");
        };
        taken.extend(names);
    }
    for line in &spec.declarations {
        let Some(name) = declaration_name(line) else {
            bail!("declarations must be single-line `export type ...;` statements");
        };
        ensure!(
            taken.insert(name.clone()),
            "`{name}` is declared or imported twice"
        );
    }
    for doc in spec
        .contexts
        .values()
        .flat_map(|c| {
            c.doc
                .iter()
                .chain(c.fields.iter().flat_map(|f| f.doc.iter()))
        })
        .chain(spec.functions.iter().filter_map(|f| f.doc.as_ref()))
    {
        ensure!(valid_doc(doc), "documentation text may not contain `*/`");
    }
    let mut plans = Vec::new();
    let mut seen = BTreeSet::new();
    for name in spec.contexts.keys() {
        ensure!(
            seen.insert(name.clone()) && !taken.contains(name),
            "context type `{name}` collides with an import, declaration or export"
        );
    }
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
            candidate.fixed.is_empty(),
            "`{}` at line {} cannot be lifted: {}",
            function.name,
            function.line,
            candidate
                .fixed
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        );
        if let Some(usage) = &function.context {
            ensure!(
                context_fields.contains_key(usage.r#type.as_str()),
                "`{}` names context `{}`, which the specification does not declare",
                function.export,
                usage.r#type
            );
            ensure!(
                valid_identifier(&usage.param),
                "`{}` is not a usable context parameter name",
                usage.param
            );
        }
        ensure!(
            (function.memo.is_none() && function.memo_read.is_none()) || function.context.is_some(),
            "`{}` declares a memo but no context",
            function.export
        );
        ensure!(
            function.memo.is_none() || function.memo_read.is_none(),
            "`{}` declares both memo and memo_read",
            function.export
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
        if let Some(memo) = &function.memo {
            for name in [&memo.compute_as, &memo.result_local] {
                ensure!(valid_identifier(name), "`{name}` is not a usable memo name");
            }
            ensure!(
                valid_type(&memo.value_type),
                "memo value type of `{}` is empty or unsafe",
                function.export
            );
            for a in &memo.reviewed_assumptions {
                ensure!(
                    valid_identifier(&a.root)
                        && !a.reason.trim().is_empty()
                        && valid_doc(&a.reason),
                    "a reviewed assumption needs a captured variable name and a reason"
                );
            }
            ensure!(
                !seen.contains(&memo.compute_as),
                "memo computation `{}` collides with an export",
                memo.compute_as
            );
            seen.insert(memo.compute_as.clone());
        }
        plans.push(Plan {
            candidate: found[0],
            spec: function,
        });
    }
    {
        let mut reserved: BTreeSet<&str> = taken.iter().map(String::as_str).collect();
        reserved.extend(ALLOWED_GLOBALS.iter().copied());
        reserved.extend([
            "Object",
            "Function",
            "Symbol",
            "globalThis",
            "Promise",
            "Error",
            "JSON",
            "String",
            "Boolean",
            "Date",
            "RegExp",
            "Set",
            "Reflect",
            "Proxy",
            "BigInt",
            "WeakMap",
        ]);
        for plan in &plans {
            let node = analysis.candidates[plan.candidate].node;
            reserved.extend(
                analysis
                    .refs
                    .iter()
                    .filter(|r| r.symbol.is_none() && within(r.span, node))
                    .map(|r| r.name.as_str()),
            );
        }
        for plan in &plans {
            for name in [
                Some(&plan.spec.export),
                plan.spec.memo.as_ref().map(|m| &m.compute_as),
            ]
            .into_iter()
            .flatten()
            {
                ensure!(
                    !reserved.contains(name.as_str()),
                    "export `{name}` collides with a global, import or declaration used in the module"
                );
            }
        }
        for name in spec.contexts.keys() {
            ensure!(
                !reserved.contains(name.as_str()),
                "context type `{name}` collides with a global used in the module"
            );
        }
    }
    let planned: BTreeMap<usize, Option<&str>> = plans
        .iter()
        .map(|p| {
            (
                p.candidate,
                p.spec.context.as_ref().map(|u| u.r#type.as_str()),
            )
        })
        .collect();
    let plan_specs: BTreeMap<usize, &FunctionSpec> =
        plans.iter().map(|p| (p.candidate, p.spec)).collect();
    // Resolve every captured read to a declared input.
    let mut resolutions: BTreeMap<usize, Resolution> = BTreeMap::new();
    for plan in &plans {
        let usage = plan.spec.context.as_ref();
        let fields = usage.map(|u| context_fields[u.r#type.as_str()].as_slice());
        let exempt = plan.spec.memo.as_ref().map(|m| m.cache.as_str()).or(plan
            .spec
            .memo_read
            .as_ref()
            .map(|m| m.cache.as_str()));
        let resolution = resolve(
            &analysis,
            plan.candidate,
            usage.zip(fields),
            &planned,
            exempt,
            &ref_by_start,
        );
        resolutions.insert(plan.candidate, resolution);
    }
    // One declared input stands for exactly one binding across every function that reads it.
    {
        let mut bound: BTreeMap<(String, usize), (oxc_semantic::SymbolId, String)> =
            BTreeMap::new();
        for plan in &plans {
            let Some(usage) = &plan.spec.context else {
                continue;
            };
            for (fi, symbol) in &resolutions[&plan.candidate].field_symbols {
                let entry = bound
                    .entry((usage.r#type.clone(), *fi))
                    .or_insert((*symbol, plan.spec.export.clone()));
                ensure!(
                    entry.0 == *symbol,
                    "context field `{}` refers to different bindings in `{}` and `{}`",
                    context_fields[usage.r#type.as_str()][*fi].def.from,
                    entry.1,
                    plan.spec.export
                );
            }
        }
    }
    // A context function must not hand declared data to a helper that edits its parameters.
    for plan in &plans {
        if plan.spec.context.is_none() {
            continue;
        }
        let deps: Vec<usize> = closure_of(&resolutions, plan.candidate)
            .into_iter()
            .filter(|d| *d != plan.candidate)
            .collect();
        for dep in deps {
            if let Some(at) =
                first_aliasing_write(&analysis, &ref_by_start, analysis.candidates[dep].node)
            {
                resolutions
                    .get_mut(&plan.candidate)
                    .expect("resolved")
                    .issues
                    .push(format!(
                        "calls `{}`, which writes through a value that may be a parameter (line {})",
                        analysis.candidates[dep].name,
                        line_of(&analysis.lines, at)
                    ));
            }
        }
    }
    for plan in &plans {
        let candidate = &analysis.candidates[plan.candidate];
        let resolution = &resolutions[&plan.candidate];
        ensure!(
            resolution.issues.is_empty(),
            "`{}` at line {} cannot be lifted: {}",
            plan.spec.name,
            plan.spec.line,
            resolution.issues.join("; ")
        );
        for dep in &resolution.deps {
            if planned.contains_key(dep) {
                continue;
            }
            let target = &analysis.candidates[*dep];
            bail!(
                "`{}` at line {} depends on `{}` at line {}, which the specification must also name{}",
                candidate.name,
                candidate.line,
                target.name,
                target.line,
                if target.closed {
                    String::new()
                } else {
                    format!(
                        " (it cannot be lifted on its own: {})",
                        target
                            .blockers
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("; ")
                    )
                }
            );
        }
    }
    // Memo caches: removed only when recomputation is provably output-neutral.
    let mut memos: BTreeMap<usize, MemoInfo> = BTreeMap::new();
    for plan in &plans {
        let Some(memo) = &plan.spec.memo else {
            continue;
        };
        let usage = plan.spec.context.as_ref().expect("checked above");
        let fields = context_fields[usage.r#type.as_str()].as_slice();
        if !resolutions[&plan.candidate].issues.is_empty() {
            continue;
        }
        match prove_memo(
            &analysis,
            plan.candidate,
            plan.spec,
            memo,
            fields,
            source,
            &resolutions,
            &plan_specs,
            &ref_by_start,
        ) {
            Ok(info) => {
                memos.insert(plan.candidate, info);
            }
            Err(issues) => resolutions
                .get_mut(&plan.candidate)
                .expect("resolved")
                .issues
                .extend(issues),
        }
    }
    let mut memo_reads: BTreeMap<usize, (Vec<Edit>, BTreeSet<u32>, MemoReadRecord)> =
        BTreeMap::new();
    for plan in &plans {
        let Some(read) = &plan.spec.memo_read else {
            continue;
        };
        let candidate = &analysis.candidates[plan.candidate];
        let mut issues = Vec::new();
        let via = plans
            .iter()
            .find(|p| p.spec.export == read.via && p.spec.memo.is_some());
        let usage = plan.spec.context.as_ref().expect("checked above");
        match (via, &candidate.seq_call) {
            (None, _) => issues.push(format!(
                "`via` names `{}`, which is not a memoizing function in the specification",
                read.via
            )),
            (_, None) => issues.push(
                "body is not `return F(), C.field` with `F` filling the cache first".to_owned(),
            ),
            (Some(via), Some(seq)) => {
                let memo = via.spec.memo.as_ref().expect("filtered");
                let info = memos.get(&via.candidate);
                let via_candidate = &analysis.candidates[via.candidate];
                let callee = ref_by_start
                    .get(&seq.callee.start)
                    .and_then(|i| analysis.refs[*i].symbol);
                if via.spec.context.as_ref().map(|u| &u.r#type) != Some(&usage.r#type) {
                    issues.push(
                        "uses a different context than the function that fills the cache"
                            .to_owned(),
                    );
                }
                if callee != Some(via_candidate.symbol) {
                    issues.push(format!(
                        "first calls `{}`, not the function that fills the cache",
                        seq.name
                    ));
                }
                if read.cache != memo.cache {
                    issues.push("reads a different cache than `via` fills".to_owned());
                }
                match info {
                    None => issues.push(format!("`{}` was not proven output-neutral", read.via)),
                    Some(info) => {
                        let mut edits = Vec::new();
                        let mut replaced = BTreeSet::new();
                        for r in analysis.refs.iter().filter(|r| {
                            r.name == read.cache
                                && within(r.span, candidate.node)
                                && r.symbol.is_some_and(|s| {
                                    analysis
                                        .scoping_symbol_spans
                                        .get(&s)
                                        .is_none_or(|d| !within(*d, candidate.node))
                                })
                        }) {
                            let field_read = !r.write
                                && within(r.span, seq.rest)
                                && analysis
                                    .chains
                                    .get(&r.span.start)
                                    .and_then(|c| c.first())
                                    .is_some_and(|(f, _)| info.fields.contains(f));
                            if r.symbol != Some(info.cache_symbol) || !field_read {
                                issues.push(format!(
                                    "cache `{}` is used at line {} other than as a field read after the fill",
                                    read.cache,
                                    line_of(&analysis.lines, r.span.start)
                                ));
                            } else {
                                edits.push(Edit {
                                    start: r.span.start as usize,
                                    end: r.span.end as usize,
                                    text: format!("{}({})", memo.compute_as, usage.param),
                                });
                                replaced.insert(r.span.start);
                            }
                        }
                        if edits.is_empty() {
                            issues.push("reads no field of the cache".to_owned());
                        }
                        if issues.is_empty() {
                            memo_reads.insert(
                                plan.candidate,
                                (
                                    edits,
                                    replaced,
                                    MemoReadRecord {
                                        cache: read.cache.clone(),
                                        via: read.via.clone(),
                                        compute: memo.compute_as.clone(),
                                    },
                                ),
                            );
                        }
                    }
                }
            }
        }
        resolutions
            .get_mut(&plan.candidate)
            .expect("resolved")
            .issues
            .extend(issues);
    }
    for plan in &plans {
        let issues = &resolutions[&plan.candidate].issues;
        ensure!(
            issues.is_empty(),
            "`{}` at line {} cannot be lifted: {}",
            plan.spec.name,
            plan.spec.line,
            issues.join("; ")
        );
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
        module.push_str(import);
        module.push('\n');
    }
    for line in &spec.declarations {
        module.push_str(line);
        module.push('\n');
    }
    for (name, def) in &spec.contexts {
        module.push_str(&format!(
            "\n/** {}Explicit inputs that replace captured variables of the original controller. */\nexport type {name} = {{\n",
            def.doc.as_deref().map(|d| format!("{d} ")).unwrap_or_default()
        ));
        for field in &def.fields {
            module.push_str(&format!(
                "  /** {}Captured `{}`. */\n  readonly {}: {};\n",
                field
                    .doc
                    .as_deref()
                    .map(|d| format!("{d} "))
                    .unwrap_or_default(),
                field.from,
                field.name,
                field.r#type.trim()
            ));
        }
        module.push_str("};\n");
    }
    let mut lifted = Vec::new();
    for plan in &plans {
        let candidate = &analysis.candidates[plan.candidate];
        let resolution = &resolutions[&plan.candidate];
        let memo = memos.get(&plan.candidate);
        let mut renames: BTreeMap<oxc_semantic::SymbolId, String> = BTreeMap::new();
        for ((_, symbol), reviewed) in candidate.params.iter().zip(&plan.spec.params) {
            renames.insert(*symbol, reviewed.name.clone());
        }
        let mut deps = Vec::new();
        for dep in &resolution.deps {
            let Some(name) = exports.get(dep) else {
                bail!(
                    "internal error: dependency `{}` was not planned",
                    analysis.candidates[*dep].name
                );
            };
            renames.insert(analysis.candidates[*dep].symbol, (*name).to_owned());
            deps.push((*name).to_owned());
        }
        renames.insert(candidate.symbol, plan.spec.export.clone());
        if let Some(inner) = candidate.inner_name {
            renames.insert(inner, plan.spec.export.clone());
        }
        let body = candidate.body;
        let mut used_locals = BTreeSet::new();
        // `name#k` renames the k-th (1-based, source order) binding of `name`
        // in the body; a bare `name` renames every binding of that name.
        let mut occurrences: BTreeMap<&str, usize> = BTreeMap::new();
        let mut declared = BTreeSet::new();
        for bind in analysis.binds.iter().filter(|b| within(b.span, body)) {
            if !declared.insert(bind.symbol) {
                continue;
            }
            let count = occurrences.entry(bind.name.as_str()).or_default();
            *count += 1;
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
        let mut hidden: BTreeSet<u32> = resolution.replaced.clone();
        if let Some(info) = memo {
            hidden.extend(info.replaced.iter().copied());
        }
        if let Some((_, replaced, _)) = memo_reads.get(&plan.candidate) {
            hidden.extend(replaced.iter().copied());
        }
        let mut kept = BTreeSet::new();
        for r in analysis.refs.iter().filter(|r| in_scope(r.span)) {
            if hidden.contains(&r.span.start) {
                continue;
            }
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
            if Some(*symbol) == candidate.inner_name {
                continue;
            }
            if let Some(previous) = targets.insert(new.as_str(), old.clone()) {
                ensure!(
                    previous == old,
                    "`{new}` would merge distinct identifiers in `{}`",
                    plan.spec.export
                );
            }
        }
        let mut introduced: Vec<&str> = Vec::new();
        if let Some(usage) = &plan.spec.context {
            introduced.push(&usage.param);
        }
        if let Some(m) = &plan.spec.memo {
            introduced.push(&m.compute_as);
            introduced.push(&m.result_local);
        }
        if let Some((_, _, record)) = memo_reads.get(&plan.candidate) {
            introduced.push(&record.compute);
        }
        let distinct: BTreeSet<&str> = introduced.iter().copied().collect();
        ensure!(
            distinct.len() == introduced.len(),
            "the context parameter, the memo computation and the memo result local must be different names in `{}`",
            plan.spec.export
        );
        for name in introduced {
            ensure!(
                !kept.contains(name) && !targets.contains_key(name),
                "introduced name `{name}` collides with an identifier in `{}`",
                plan.spec.export
            );
        }
        let mut edits: Vec<Edit> = Vec::new();
        for r in analysis.refs.iter().filter(|r| within(r.span, body)) {
            if let Some(new) = r.symbol.and_then(|s| renames.get(&s)) {
                edits.push(Edit {
                    start: r.span.start as usize,
                    end: r.span.end as usize,
                    text: new.clone(),
                });
            }
        }
        for b in analysis.binds.iter().filter(|b| within(b.span, body)) {
            if let Some(new) = renames.get(&b.symbol) {
                edits.push(Edit {
                    start: b.span.start as usize,
                    end: b.span.end as usize,
                    text: new.clone(),
                });
            }
        }
        for e in &edits {
            ensure!(
                !analysis
                    .shorthand
                    .iter()
                    .any(|s| (e.start as u32) >= s.start && (e.end as u32) <= s.end),
                "renaming inside a shorthand property in `{}` is not supported",
                plan.spec.export
            );
        }
        for e in &resolution.edits {
            ensure!(
                !analysis
                    .shorthand
                    .iter()
                    .any(|s| (e.start as u32) >= s.start && (e.end as u32) <= s.end),
                "replacing a captured read inside a shorthand property in `{}` is not supported",
                plan.spec.export
            );
        }
        let mut extra: Vec<&Edit> = Vec::new();
        extra.extend(edits.iter());
        extra.extend(resolution.edits.iter());
        if let Some(info) = memo {
            extra.extend(info.edits.iter());
        }
        let read_record = memo_reads.get(&plan.candidate);
        if let Some((read_edits, _, _)) = read_record {
            extra.extend(read_edits.iter());
        }
        let node_text = &source[candidate.node.start as usize..candidate.node.end as usize];
        let node_hash = fingerprint(node_text.as_bytes());
        let mut params = plan
            .spec
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name, p.r#type.trim()))
            .collect::<Vec<_>>();
        if let Some(usage) = &plan.spec.context {
            params.push(format!("{}: {}", usage.param, usage.r#type));
        }
        let params = params.join(", ");
        let doc = plan
            .spec
            .doc
            .as_deref()
            .map(|d| format!("{d} "))
            .unwrap_or_default();
        let provenance = format!(
            "Lifted from `{}` (line {}, bytes {}..{}, sha256 {}).",
            candidate.name,
            candidate.line,
            candidate.node.start,
            candidate.node.end,
            &node_hash[..12]
        );
        let what = plan.spec.export.as_str();
        if let (Some(ma), Some(_), Some(spec_memo), Some(usage)) = (
            candidate.memo.as_ref().ok(),
            memo,
            plan.spec.memo.as_ref(),
            plan.spec.context.as_ref(),
        ) {
            let mut compute_edits: Vec<&Edit> = extra.clone();
            let strip_fill = Edit {
                start: ma.assign_target.start as usize,
                end: ma.assign_rhs.start as usize,
                text: "return ".to_owned(),
            };
            compute_edits.push(&strip_fill);
            let key_from = {
                let ls = line_start(source, ma.key_stmt.start as usize);
                if source[ls..ma.key_stmt.start as usize].trim().is_empty() {
                    ls
                } else {
                    ma.key_stmt.start as usize
                }
            };
            let key_piece = apply(
                source,
                key_from,
                ma.key_stmt.end as usize,
                &compute_edits,
                &analysis.literals,
                what,
            )?;
            let block_piece = apply(
                source,
                ma.block.start as usize + 1,
                ma.assign_stmt.end as usize,
                &compute_edits,
                &analysis.literals,
                what,
            )?;
            let rest_piece = apply(
                source,
                ma.rest.start as usize,
                ma.rest.end as usize,
                &extra,
                &analysis.literals,
                what,
            )?;
            module.push_str(&format!(
                "\n/** {doc}The memoized computation of `{}`, extracted so the cache can be dropped. {provenance} */\nexport function {}({}: {}): {} {{\n{}\n{}\n}}\n",
                plan.spec.export,
                spec_memo.compute_as,
                usage.param,
                usage.r#type,
                spec_memo.value_type.trim(),
                reindent_block(&key_piece).replace(KEEP_BREAK, "\n"),
                reindent_block(&block_piece).replace(KEEP_BREAK, "\n"),
            ));
            module.push_str(&format!(
                "\n/** {doc}{provenance} */\nexport function {}({}): {} {{\n  const {} = {}({});\n{}\n}}\n",
                plan.spec.export,
                params,
                plan.spec.returns.trim(),
                spec_memo.result_local,
                spec_memo.compute_as,
                usage.param,
                reindent_block(&rest_piece).replace(KEEP_BREAK, "\n"),
            ));
        } else {
            let text = apply(
                source,
                body.start as usize,
                body.end as usize,
                &extra,
                &analysis.literals,
                what,
            )?;
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
            module.push_str(&format!(
                "\n/** {doc}{provenance} */\nexport function {}({}): {} {{\n{}\n}}\n",
                plan.spec.export,
                params,
                plan.spec.returns.trim(),
                rendered.replace(KEEP_BREAK, "\n")
            ));
        }
        let usage = plan.spec.context.as_ref();
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
            context: usage.map(|u| ContextRecord {
                r#type: u.r#type.clone(),
                param: u.param.clone(),
                reads: resolution
                    .reads
                    .iter()
                    .map(|fi| context_fields[u.r#type.as_str()][*fi].def.name.clone())
                    .collect(),
            }),
            memo: memos.remove(&plan.candidate).map(|m| m.proof),
            memo_read: memo_reads.remove(&plan.candidate).map(|(_, _, r)| r),
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
    {
        let arena = Allocator::default();
        let Some(program) = ts_program(&arena, &module) else {
            bail!("the emitted module does not parse as TypeScript");
        };
        let mut only = TsOnly(Vec::new());
        only.visit_program(&program);
        ensure!(
            only.0.is_empty(),
            "TypeScript reads the original code differently (it finds {}); the lift is refused",
            only.0.join(", ")
        );
    }
    let manifest = Manifest {
        schema_version: 1,
        tool_version: env!("CARGO_PKG_VERSION"),
        source: spec.source.to_string_lossy().into_owned(),
        source_sha256: digest,
        module_sha256: fingerprint(module.as_bytes()),
        contexts: spec.contexts.clone(),
        functions: lifted,
        unlifted,
    };
    write_pair(
        module_out,
        module.as_bytes(),
        manifest_out,
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
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
mod tests;
