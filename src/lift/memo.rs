//! Stability and memo-cache proofs.
use super::*;
use anyhow::Result;
use oxc_span::Span;
use std::collections::{BTreeMap, BTreeSet};

/// Methods that read their receiver and hand back no element of it (so no alias). Any other
/// method called on a reviewed root must be listed in its assumption.
pub(super) const READ_METHODS: &[&str] = &[
    "map",
    "forEach",
    "some",
    "every",
    "reduce",
    "reduceRight",
    "indexOf",
    "lastIndexOf",
    "includes",
    "join",
    "has",
    "toString",
    "toFixed",
];
/// Why a captured root cannot change while a cache is alive, or why that is not shown.
pub(super) struct Stability {
    pub(super) evidence: String,
    pub(super) assumed: bool,
}
pub(super) fn root_stability(
    analysis: &Analysis,
    ref_by_start: &BTreeMap<u32, usize>,
    symbol: oxc_semantic::SymbolId,
    name: &str,
    declared: &[(&[String], bool)],
    assumption: Option<&ReviewedAssumption>,
) -> Result<Stability, String> {
    let mut reads = 0usize;
    let mut members = BTreeSet::new();
    let mut escapes = Vec::new();
    let mut assumed = false;
    let mut called: BTreeSet<String> = BTreeSet::new();
    let mut notes: Vec<String> = Vec::new();
    // The binding itself must be one fixed variable.
    if analysis.redeclared.contains(&symbol) {
        return Err(format!("`{name}` is declared more than once"));
    }
    if let Some(w) = analysis
        .refs
        .iter()
        .find(|r| r.symbol == Some(symbol) && r.write)
    {
        return Err(format!(
            "`{name}` is reassigned at line {}",
            line_of(&analysis.lines, w.span.start)
        ));
    }
    // `Object.freeze` at every level over primitives is the one thing that proves a value
    // cannot change (as long as the global `Object.freeze` is intact); everything else needs a
    // reviewer's word. Either way the direct checks below still run.
    let shape = analysis.frozen_shapes.get(&symbol);
    if let Some(shape) = shape {
        for (path, _) in declared.iter().filter(|(p, _)| !p.is_empty()) {
            if !shape.primitive_at(path) {
                return Err(format!(
                    "the declared path `{name}.{}` is not an own data property of the frozen literal ending in a primitive, so it could be inherited and patched",
                    path.join(".")
                ));
            }
        }
        notes.push("a literal frozen with Object.freeze at every level; every declared path is an own data property ending in a primitive. Conditional on the global Object.freeze being intact when the literal was created".to_owned());
    } else {
        let Some(assumption) = assumption else {
            return Err(format!(
                "`{name}` is not an Object.freeze'd literal at every level, so nothing here proves it cannot change; list it under `reviewed_assumptions` with a reason if a reviewer accepts that"
            ));
        };
        assumed = true;
        let _ = assumption;
        notes.push(
            if analysis.imports.contains(&symbol) {
                "an imported binding whose value is authored elsewhere and not inspected"
            } else {
                "initialized with something other than a deeply frozen literal"
            }
            .to_owned(),
        );
    }
    for r in analysis.refs.iter().filter(|r| r.symbol == Some(symbol)) {
        let line = line_of(&analysis.lines, r.span.start);
        if r.write {
            return Err(format!("`{name}` is reassigned at line {line}"));
        }
        reads += 1;
        if !analysis.member_roots.contains(&r.span.start) {
            escapes.push(line);
        }
        let chain = analysis
            .chains
            .get(&r.span.start)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if let Some((first, _)) = chain.first() {
            members.insert(first.clone());
        }
        // A value on the way to a declared path (`geom.config` for `geom.config.size`)
        // can be aliased and mutated, which the declared path would then reflect.
        for (path, _) in declared {
            if chain.len() < path.len()
                && chain
                    .iter()
                    .zip(path.iter())
                    .all(|((have, _), want)| have == want)
            {
                return Err(format!(
                    "`{name}{}{}` is used as a value at line {}, which can alias the declared path",
                    if chain.is_empty() { "" } else { "." },
                    chain
                        .iter()
                        .map(|(p, _)| p.as_str())
                        .collect::<Vec<_>>()
                        .join("."),
                    line
                ));
            }
        }
    }
    for root in &analysis.member_writes {
        if let Some(&ri) = ref_by_start.get(&root.start)
            && analysis.refs[ri].symbol == Some(symbol)
        {
            return Err(format!(
                "`{name}` is written through at line {}",
                line_of(&analysis.lines, root.start)
            ));
        }
    }
    for call in &analysis.method_calls {
        let Some(&ri) = ref_by_start.get(&call.root.start) else {
            continue;
        };
        if analysis.refs[ri].symbol != Some(symbol) {
            continue;
        }
        let line = line_of(&analysis.lines, call.root.start);
        if !READ_METHODS.contains(&call.method.as_str()) {
            called.insert(call.method.clone());
        }
        let callbacks: Vec<Span> = call
            .args
            .map(|args| {
                analysis
                    .fn_spans
                    .iter()
                    .filter(|f| within(**f, args))
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        for callback in &callbacks {
            let lo = analysis
                .member_writes
                .partition_point(|s| s.start < callback.start);
            for root in analysis.member_writes[lo..]
                .iter()
                .take_while(|s| s.start < callback.end)
            {
                let fresh_local = ref_by_start
                    .get(&root.start)
                    .and_then(|i| analysis.refs[*i].symbol)
                    .is_some_and(|s| analysis.fresh.contains(&s));
                if !fresh_local {
                    return Err(format!(
                        "a callback passed to `{name}.{}` at line {line} writes through a value (line {})",
                        call.method,
                        line_of(&analysis.lines, root.start)
                    ));
                }
            }
        }
    }
    if !escapes.is_empty() {
        return Err(format!(
            "`{name}` is used as a bare value (aliasable) at line(s) {}",
            escapes
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    // Values that may be aliased and edited elsewhere: arguments, initializers, assignments,
    // returns, spreads and literal members. A leaf the frozen shape or the declared primitive
    // fields vouch for is only a number or string.
    for root in &analysis.escaping_uses {
        let Some(&ri) = ref_by_start.get(&root.start) else {
            continue;
        };
        if analysis.refs[ri].symbol != Some(symbol) {
            continue;
        }
        let chain: Vec<String> = analysis
            .chains
            .get(&root.start)
            .map(|c| c.iter().map(|(p, _)| p.clone()).collect())
            .unwrap_or_default();
        let primitive = shape.is_some_and(|sh| sh.primitive_at(&chain))
            || assumption.is_some_and(|a| {
                a.primitives.iter().any(|p| {
                    let path: Vec<&str> = p.split('.').collect();
                    chain.len() >= path.len() && chain.iter().zip(&path).all(|(c, q)| c == q)
                })
            })
            || declared.iter().any(|(path, prim)| {
                *prim
                    && !path.is_empty()
                    && chain.len() >= path.len()
                    && chain[..path.len()] == **path
            });
        if !primitive {
            return Err(format!(
                "a value reached through `{name}` is passed on, assigned, returned or stored at line {}, where it can be aliased and edited",
                line_of(&analysis.lines, root.start)
            ));
        }
    }
    if shape.is_none()
        && let Some(a) = assumption
    {
        let listed: BTreeSet<String> = a.methods.iter().cloned().collect();
        if listed != called {
            return Err(format!(
                "the methods called on `{name}` ({}) differ from the ones the reviewed assumption lists ({})",
                called.iter().cloned().collect::<Vec<_>>().join(", "),
                listed.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    if !called.is_empty() {
        notes.push(format!(
            "methods called on it whose effect on the receiver is unknown and therefore covered by the assumption: {}",
            called.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if let (true, Some(a)) = (shape.is_none(), assumption)
        && !a.primitives.is_empty()
    {
        notes.push(format!(
            "the reviewer vouches these member paths are primitives: {}",
            a.primitives.join(", ")
        ));
    }
    if let (true, Some(a)) = (shape.is_none(), assumption) {
        notes.push(format!(
            "ACCEPTED ON A REVIEWER'S STATED REASON, NOT PROVEN: {}",
            a.reason
        ));
    }
    Ok(Stability {
        evidence: format!(
            "{reads} reference(s) in this module; none reassigns `{name}`, writes through it directly, passes an inline callback that writes, or uses it as a bare value (these direct checks do not see aliases or helpers); members touched: {}; {}",
            if members.is_empty() {
                "none directly (indexed or called only)".to_owned()
            } else {
                members.into_iter().collect::<Vec<_>>().join(", ")
            },
            notes.join("; ")
        ),
        assumed,
    })
}

pub(super) struct MemoInfo {
    pub(super) cache_symbol: oxc_semantic::SymbolId,
    pub(super) fields: Vec<String>,
    pub(super) proof: MemoProof,
    pub(super) edits: Vec<Edit>,
    pub(super) replaced: BTreeSet<u32>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prove_memo(
    analysis: &Analysis,
    ci: usize,
    spec: &FunctionSpec,
    memo: &MemoSpec,
    fields: &[FieldDef<'_>],
    source: &str,
    resolutions: &BTreeMap<usize, Resolution>,
    plan_specs: &BTreeMap<usize, &FunctionSpec>,
    ref_by_start: &BTreeMap<u32, usize>,
) -> Result<MemoInfo, Vec<String>> {
    let candidate = &analysis.candidates[ci];
    let ma = candidate
        .memo
        .as_ref()
        .map_err(|why| vec![format!("is not a write-back memo function: {why}")])?;
    let mut issues = Vec::new();
    if ma.cache != memo.cache {
        issues.push(format!(
            "the memo cache is `{}`, not `{}`",
            ma.cache, memo.cache
        ));
    }
    if !ma.key_pure {
        issues.push("the key expression is not a pure expression of inputs".to_owned());
    }
    if ma.block_returns {
        issues.push(
            "the guarded block contains a `return`, which the extracted computation would turn into a different value".to_owned(),
        );
    }
    if !candidate.params.is_empty() {
        issues.push(
            "the memoizing function has parameters, which the cache key does not cover".to_owned(),
        );
    }
    if ma
        .key_symbol
        .is_some_and(|k| analysis.redeclared.contains(&k))
    {
        issues.push("the key variable is declared more than once".to_owned());
    }
    let key_name = ma
        .key_symbol
        .and_then(|k| analysis.binds.iter().find(|b| b.symbol == k))
        .map(|b| b.name.clone())
        .unwrap_or_default();
    if let (Some(key), Some(stored)) = (ma.key_symbol, ma.key_value_ref)
        && ref_by_start
            .get(&stored.start)
            .and_then(|i| analysis.refs[*i].symbol)
            != Some(key)
    {
        issues.push(
            "the `key` stored in the cached object is not the key variable (a shadowing declaration inside the guarded block?)".to_owned(),
        );
    }
    for name in &ma.block_decls {
        if *name == ma.cache || *name == key_name {
            issues.push(format!(
                "the guarded block declares `{name}`, the name of the key or the cache, which would duplicate or shadow it in the extracted computation"
            ));
        }
    }
    if let Some(key) = ma.key_symbol
        && let Some(w) = analysis
            .refs
            .iter()
            .find(|r| r.symbol == Some(key) && r.write)
    {
        issues.push(format!(
            "the key variable is reassigned at line {}",
            line_of(&analysis.lines, w.span.start)
        ));
    }
    let sym_at = |span: Span| {
        ref_by_start
            .get(&span.start)
            .and_then(|i| analysis.refs[*i].symbol)
    };
    let cache_symbol = sym_at(ma.test_refs[0]);
    let Some(cache_symbol) = cache_symbol else {
        return Err(vec![format!(
            "cache `{}` is not a captured variable",
            ma.cache
        )]);
    };
    if [ma.test_refs[1], ma.assign_target]
        .iter()
        .any(|s| sym_at(*s) != Some(cache_symbol))
    {
        issues.push("the guard and the fill refer to different bindings".to_owned());
    }
    if analysis
        .scoping_symbol_spans
        .get(&cache_symbol)
        .is_none_or(|d| within(*d, candidate.node))
    {
        issues.push(format!("cache `{}` is not captured", ma.cache));
    }
    if analysis.redeclared.contains(&cache_symbol) {
        issues.push(format!("cache `{}` is declared more than once", ma.cache));
    }
    if analysis.var_declared.contains(&cache_symbol) {
        issues.push(format!(
            "cache `{}` is declared with `var`, so a repeated declaration with an initializer would reset it unseen",
            ma.cache
        ));
    }
    if analysis.exported.contains(&cache_symbol) {
        issues.push(format!("cache `{}` is exported from the module", ma.cache));
    }
    // Every reference to the cache inside the function is the guard, the fill,
    // or a field read after the guard.
    let resolution = &resolutions[&ci];
    for r in analysis
        .refs
        .iter()
        .filter(|r| r.symbol == Some(cache_symbol) && within(r.span, candidate.node))
    {
        let start = r.span.start;
        let guard =
            ma.test_refs.iter().any(|s| s.start == start) || ma.assign_target.start == start;
        let rest_read = within(r.span, ma.rest)
            && !r.write
            && analysis
                .chains
                .get(&start)
                .and_then(|c| c.first())
                .is_some_and(|(field, _)| ma.fields.contains(field));
        if !guard && !rest_read {
            issues.push(format!(
                "cache `{}` is referenced at line {} outside the guard, the fill and field reads after the guard",
                ma.cache,
                line_of(&analysis.lines, start)
            ));
        }
    }
    // Everywhere else in the module the cache may only be reset, or read by the function that
    // reads it after the fill: an alias (`const c = cache; c.mid = 1`) would let the original
    // diverge from a recomputation.
    let reader_nodes: Vec<Span> = plan_specs
        .iter()
        .filter(|(_, p)| {
            p.memo_read
                .as_ref()
                .is_some_and(|m| m.cache == memo.cache && m.via == spec.export)
        })
        .map(|(c, _)| analysis.candidates[*c].node)
        .collect();
    for r in analysis
        .refs
        .iter()
        .filter(|r| r.symbol == Some(cache_symbol) && !within(r.span, candidate.node))
    {
        let reset = analysis
            .assigns
            .iter()
            .any(|a| a.target.start == r.span.start && a.plain && a.null);
        let field_read = !r.write
            && reader_nodes.iter().any(|n| within(r.span, *n))
            && analysis
                .chains
                .get(&r.span.start)
                .and_then(|c| c.first())
                .is_some_and(|(field, _)| ma.fields.contains(field));
        if !reset && !field_read {
            issues.push(format!(
                "cache `{}` is used at line {} other than as a reset or as a field read in the function that reads it after the fill (it could be aliased and edited)",
                ma.cache,
                line_of(&analysis.lines, r.span.start)
            ));
        }
    }
    // The code after the guard sees only the cache, not the computation's locals.
    for r in analysis.refs.iter().filter(|r| within(r.span, ma.rest)) {
        let local_to_computation = r.symbol.is_some_and(|s| {
            analysis.scoping_symbol_spans.get(&s).is_some_and(|d| {
                within(*d, candidate.node)
                    && !within(*d, ma.rest)
                    && (within(*d, ma.key_stmt) || within(*d, ma.block))
            })
        });
        if local_to_computation {
            issues.push(format!(
                "code after the cache guard uses `{}`, which only exists inside the memoized computation",
                r.name
            ));
        }
    }
    // The cache object itself must not be edited in place.
    let member_written = |symbol: oxc_semantic::SymbolId| {
        analysis.member_writes.iter().find(|root| {
            ref_by_start
                .get(&root.start)
                .is_some_and(|i| analysis.refs[*i].symbol == Some(symbol))
        })
    };
    if let Some(root) = member_written(cache_symbol) {
        issues.push(format!(
            "cache `{}` is edited in place at line {}",
            ma.cache,
            line_of(&analysis.lines, root.start)
        ));
    }
    // Key expression: only declared inputs.
    let mut key_fields = BTreeSet::new();
    for r in analysis.refs.iter().filter(|r| within(r.span, ma.key_expr)) {
        match resolution.ref_field.get(&r.span.start) {
            Some(fi) => {
                key_fields.insert(*fi);
            }
            None => issues.push(format!(
                "the key reads `{}`, which is not a declared context field",
                r.name
            )),
        }
    }
    if key_fields.is_empty() {
        issues.push("the key reads no declared input".to_owned());
    }
    // The key must tell different inputs apart: a template of numeric fields separated by text
    // that cannot occur inside a printed number.
    match &ma.key_parts {
        Some((quasis, expressions)) if quasis.len() == expressions.len() + 1 => {
            for span in expressions {
                let bare = resolution.ref_field.get(&span.start).is_some_and(|fi| {
                    fields[*fi].def.r#type.trim() == "number"
                        && resolution
                            .edits
                            .iter()
                            .any(|e| e.start == span.start as usize && e.end == span.end as usize)
                });
                if !bare {
                    issues.push(
                        "the key template interpolates something other than a bare numeric declared field".to_owned(),
                    );
                }
            }
            for separator in quasis.iter().skip(1).take(quasis.len().saturating_sub(2)) {
                if !separator.chars().any(|c| !"0123456789.-+eEInfitNay".contains(c)) {
                    issues.push(
                        "the key joins fields with text that can occur inside a printed number, so different inputs could share a key".to_owned(),
                    );
                }
            }
        }
        _ => issues.push(
            "the key is not a template literal of numeric declared fields, so it is not shown to tell inputs apart".to_owned(),
        ),
    }
    // Writes to the cache anywhere in the module.
    let mut cache_writes = Vec::new();
    let mut cache_resets: BTreeSet<(u32, usize)> = BTreeSet::new();
    for a in &analysis.assigns {
        if sym_at(a.target) != Some(cache_symbol) {
            continue;
        }
        let line = line_of(&analysis.lines, a.whole.start);
        if a.whole == ma.assign_expr {
            cache_writes.push(WriteSite {
                line,
                kind: "fill inside the memoized computation".to_owned(),
            });
        } else if a.plain && a.null {
            cache_writes.push(WriteSite {
                line,
                kind: "reset to null".to_owned(),
            });
            if let Some((seq, index)) = a.seq {
                cache_resets.insert((seq, index));
            }
        } else {
            issues.push(format!(
                "cache `{}` is assigned something other than null or the memoized value at line {line}",
                ma.cache
            ));
        }
    }
    let assign_targets: BTreeSet<u32> = analysis.assigns.iter().map(|a| a.target.start).collect();
    for r in analysis
        .refs
        .iter()
        .filter(|r| r.symbol == Some(cache_symbol) && r.write)
    {
        if !assign_targets.contains(&r.span.start) {
            issues.push(format!(
                "cache `{}` is written at line {} in a way the proof does not model",
                ma.cache,
                line_of(&analysis.lines, r.span.start)
            ));
        }
    }
    // Everything the computation reads, through its dependencies.
    let closure = closure_of(resolutions, ci);
    let mut used = BTreeSet::new();
    for member in &closure {
        if *member != ci
            && plan_specs
                .get(member)
                .is_some_and(|s| s.memo.is_some() || s.memo_read.is_some())
        {
            issues.push(format!(
                "depends on `{}`, which has its own cache",
                analysis.candidates[*member].name
            ));
        }
        if let Some(res) = resolutions.get(member) {
            used.extend(res.reads.iter().copied());
        }
    }
    if !analysis.null_init.contains(&cache_symbol) {
        issues.push(format!(
            "cache `{}` is not declared empty (`null` or no initializer)",
            ma.cache
        ));
    }
    let mut stable_inputs = Vec::new();
    let mut coupled = Vec::new();
    let mut sampler_side_effects: Vec<String> = Vec::new();
    let mut reviewed: Vec<String> = Vec::new();
    let mut used_assumptions: BTreeSet<String> = BTreeSet::new();
    let mut stable_roots = BTreeSet::new();
    let mut symbols: BTreeMap<usize, oxc_semantic::SymbolId> = BTreeMap::new();
    for member in &closure {
        if let Some(res) = resolutions.get(member) {
            symbols.extend(res.field_symbols.iter().map(|(k, v)| (*k, *v)));
        }
    }
    for fi in used.difference(&key_fields) {
        let field = &fields[*fi];
        let Some(symbol) = symbols.get(fi).copied() else {
            continue;
        };
        if field.call {
            let Some(link) = memo.coupled.iter().find(|c| c.sampler == field.def.from) else {
                issues.push(format!(
                    "sampled input `{}` has no coupled cache declared",
                    field.def.from
                ));
                continue;
            };
            let Some(sampler) = analysis.candidates.iter().position(|c| c.symbol == symbol) else {
                issues.push(format!("`{}` is not a function in this module", field.root));
                continue;
            };
            let node = analysis.candidates[sampler].node;
            let (sampler_cache, other_writes) = match sampler_cache(analysis, sampler) {
                Ok(found) => found,
                Err(why) => {
                    issues.push(format!(
                        "`{}` is not a guarded cache getter: {why}",
                        field.root
                    ));
                    continue;
                }
            };
            if analysis.candidates[sampler].sampler.as_deref() != Some(link.cache.as_str()) {
                issues.push(format!(
                    "`{}` returns a cache other than `{}`",
                    field.root, link.cache
                ));
                continue;
            }
            if let Some(root) = member_written(sampler_cache) {
                issues.push(format!(
                    "`{}` is edited in place at line {}",
                    link.cache,
                    line_of(&analysis.lines, root.start)
                ));
            }
            for r in analysis
                .refs
                .iter()
                .filter(|r| r.symbol == Some(sampler_cache) && !within(r.span, node))
            {
                let reset = analysis
                    .assigns
                    .iter()
                    .any(|a| a.target.start == r.span.start && a.plain && a.null);
                if !reset {
                    issues.push(format!(
                        "`{}` is used at line {} other than as a reset or inside `{}` (it could be aliased and edited)",
                        link.cache,
                        line_of(&analysis.lines, r.span.start),
                        field.root
                    ));
                }
            }
            if !other_writes.is_empty() {
                sampler_side_effects.push(format!(
                    "`{}` also writes {}; this proof does not model what reads them",
                    field.root,
                    other_writes
                        .iter()
                        .map(|n| format!("`{n}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if analysis.redeclared.contains(&sampler_cache) {
                issues.push(format!("`{}` is declared more than once", link.cache));
            }
            if analysis.var_declared.contains(&sampler_cache) {
                issues.push(format!(
                    "`{}` is declared with `var`, so a repeated declaration with an initializer would reset it unseen",
                    link.cache
                ));
            }
            if analysis.exported.contains(&sampler_cache) {
                issues.push(format!("`{}` is exported from the module", link.cache));
            }
            // The sampled object must not be edited or leaked through the sampler's result.
            let sampler_symbol = analysis.candidates[sampler].symbol;
            let aliases: Vec<oxc_semantic::SymbolId> = analysis
                .call_inits
                .iter()
                .filter(|(_, callee)| {
                    ref_by_start
                        .get(callee)
                        .is_some_and(|i| analysis.refs[*i].symbol == Some(sampler_symbol))
                })
                .map(|(sym, _)| *sym)
                .collect();
            for root in &analysis.member_writes {
                let Some(&ri) = ref_by_start.get(&root.start) else {
                    continue;
                };
                let rooted = analysis.refs[ri]
                    .symbol
                    .is_some_and(|sym| sym == sampler_symbol || aliases.contains(&sym));
                if rooted {
                    issues.push(format!(
                        "the object `{}()` returns is edited in place at line {}",
                        field.root,
                        line_of(&analysis.lines, root.start)
                    ));
                }
            }
            for r in analysis
                .refs
                .iter()
                .filter(|r| r.symbol == Some(sampler_symbol))
            {
                if analysis.escaping_calls.contains(&r.span.start) {
                    issues.push(format!(
                        "the result of `{}()` is passed on, returned or stored at line {}, where it can be edited",
                        field.root,
                        line_of(&analysis.lines, r.span.start)
                    ));
                }
            }
            for root in &analysis.escaping_uses {
                let Some(&ri) = ref_by_start.get(&root.start) else {
                    continue;
                };
                if analysis.refs[ri]
                    .symbol
                    .is_some_and(|sym| aliases.contains(&sym))
                    && analysis
                        .chains
                        .get(&root.start)
                        .is_none_or(|c| c.is_empty())
                {
                    issues.push(format!(
                        "a local holding the result of `{}()` is passed on or stored as a whole at line {}",
                        field.root,
                        line_of(&analysis.lines, root.start)
                    ));
                }
            }
            // `U` itself may be named only by the guard, the fill and the final return.
            for r in analysis
                .refs
                .iter()
                .filter(|r| r.symbol == Some(sampler_cache) && within(r.span, node))
            {
                let allowed = analysis.candidates[sampler]
                    .sampler_refs
                    .iter()
                    .any(|s| s.start == r.span.start)
                    || (analysis
                        .assigns
                        .iter()
                        .any(|a| a.target.start == r.span.start)
                        && !analysis.chained_targets.contains(&r.span.start));
                if !allowed {
                    issues.push(format!(
                        "`{}` is used inside `{}` at line {} other than by the guard, the fill and the final return (it could leak the sampled object)",
                        link.cache,
                        field.root,
                        line_of(&analysis.lines, r.span.start)
                    ));
                }
            }
            if !analysis.null_init.contains(&sampler_cache) {
                issues.push(format!(
                    "`{}` is not declared empty (`null` or no initializer)",
                    link.cache
                ));
            }
            let mut fills = Vec::new();
            let mut paired = Vec::new();
            for a in analysis
                .assigns
                .iter()
                .filter(|a| sym_at(a.target) == Some(sampler_cache))
            {
                let line = line_of(&analysis.lines, a.whole.start);
                if a.plain && a.null {
                    if a.seq.is_some_and(|(seq, i)| {
                        cache_resets.contains(&(seq, i + 1))
                            || (i > 0 && cache_resets.contains(&(seq, i - 1)))
                    }) {
                        paired.push(line);
                    } else {
                        issues.push(format!(
                            "`{}` is reset at line {line} without resetting `{}` in the same expression",
                            link.cache, memo.cache
                        ));
                    }
                } else if within(a.whole, node)
                    && a.plain
                    && a.literal
                    && a.values_ok
                    && !analysis
                        .fn_spans
                        .iter()
                        .any(|f| *f != node && within(a.whole, *f) && within(*f, node))
                    && a.value_roots.iter().all(|root| {
                        // A bare local of the sampler that only ever holds a number computed
                        // there; anything else could be (or alias) shared data.
                        let Some(sym) = ref_by_start
                            .get(&root.start)
                            .and_then(|i| analysis.refs[*i].symbol)
                        else {
                            // A global: only the constants `undefined`, `NaN` and `Infinity`.
                            return analysis
                                .refs
                                .iter()
                                .find(|r| r.span.start == root.start)
                                .is_some_and(|r| {
                                    matches!(r.name.as_str(), "undefined" | "NaN" | "Infinity")
                                });
                        };
                        analysis
                            .scoping_symbol_spans
                            .get(&sym)
                            .is_some_and(|d| within(*d, node))
                            && analysis
                                .chains
                                .get(&root.start)
                                .is_none_or(|c| c.is_empty())
                            && analysis.decl_numeric.get(&sym).copied().unwrap_or(false)
                            && analysis
                                .assigns
                                .iter()
                                .filter(|w| sym_at(w.target) == Some(sym))
                                .all(|w| w.numeric)
                            && !analysis.refs.iter().any(|w| {
                                w.symbol == Some(sym)
                                    && w.write
                                    && !analysis
                                        .assigns
                                        .iter()
                                        .any(|x| x.target.start == w.span.start)
                            })
                    })
                    && analysis
                        .guards
                        .iter()
                        .any(|(n, g)| *n == link.cache && within(a.whole, *g))
                {
                    fills.push(WriteSite {
                        line,
                        kind: format!("filled inside `{}`", field.root),
                    });
                } else if within(a.whole, node) {
                    issues.push(format!(
                        "`{}` is not filled by an object or array literal inside its `if (!{})` guard at line {line}; a conditional or compound assignment (`&&=`, `+=`), a nested object/array, a value that is not arithmetic, a literal or a bare local, a fill inside a nested function (a deferred refill), or a value that is not a number computed in the sampler would let the sample change without the cache being reset",
                        link.cache, link.cache
                    ));
                } else {
                    issues.push(format!(
                        "`{}` is assigned outside `{}` at line {line}",
                        link.cache, field.root
                    ));
                }
            }
            for r in analysis
                .refs
                .iter()
                .filter(|r| r.symbol == Some(sampler_cache) && r.write)
            {
                if !assign_targets.contains(&r.span.start) {
                    issues.push(format!(
                        "`{}` is written at line {} in a way the proof does not model",
                        link.cache,
                        line_of(&analysis.lines, r.span.start)
                    ));
                }
            }
            if paired.is_empty() {
                issues.push(format!(
                    "`{}` is never reset together with `{}`",
                    link.cache, memo.cache
                ));
            }
            coupled.push(CoupledProof {
                cache: link.cache.clone(),
                sampler: field.def.from.clone(),
                fills,
                resets_paired_with_cache_reset: paired,
            });
        } else if stable_roots.insert(field.root.clone()) {
            let declared: Vec<(&[String], bool)> = fields
                .iter()
                .filter(|g| g.root == field.root && !g.call)
                .map(|g| {
                    (
                        g.props.as_slice(),
                        matches!(g.def.r#type.trim(), "number" | "string" | "boolean"),
                    )
                })
                .collect();
            if analysis.exported.contains(&symbol) {
                issues.push(format!(
                    "input `{}` is exported from the module, so other code can reach it",
                    field.def.from
                ));
            }
            let assumption = memo
                .reviewed_assumptions
                .iter()
                .find(|a| a.root == field.root);
            match root_stability(
                analysis,
                ref_by_start,
                symbol,
                &field.root,
                &declared,
                assumption,
            ) {
                Ok(found) => {
                    if found.assumed
                        && let Some(a) = assumption
                    {
                        used_assumptions.insert(a.root.clone());
                        reviewed.push(format!("`{}`: {}", a.root, a.reason));
                    }
                    stable_inputs.push(StableInput {
                        field: field.def.from.clone(),
                        evidence: found.evidence,
                    });
                }
                Err(why) => issues.push(format!(
                    "input `{}` is not covered by the key and cannot be shown stable: {why}",
                    field.def.from
                )),
            }
        } else {
            stable_inputs.push(StableInput {
                field: field.def.from.clone(),
                evidence: format!("shares the proven-stable root `{}`", field.root),
            });
        }
    }
    for a in &memo.reviewed_assumptions {
        if !used_assumptions.contains(&a.root) {
            issues.push(format!(
                "the reviewed assumption for `{}` is not needed (the tool proved it) or names no input of the computation",
                a.root
            ));
        }
    }
    for link in &memo.coupled {
        if !used.iter().any(|fi| fields[*fi].def.from == link.sampler) {
            issues.push(format!(
                "coupled cache `{}` names sampler `{}`, which the computation does not read",
                link.cache, link.sampler
            ));
        }
    }
    if !issues.is_empty() {
        return Err(issues);
    }
    let mut edits = Vec::new();
    let mut replaced: BTreeSet<u32> = ma.test_refs.iter().map(|s| s.start).collect();
    replaced.insert(ma.assign_target.start);
    for r in analysis
        .refs
        .iter()
        .filter(|r| r.symbol == Some(cache_symbol) && within(r.span, ma.rest) && !r.write)
    {
        edits.push(Edit {
            start: r.span.start as usize,
            end: r.span.end as usize,
            text: memo.result_local.clone(),
        });
        replaced.insert(r.span.start);
    }
    Ok(MemoInfo {
        cache_symbol,
        fields: ma.fields.clone(),
        proof: MemoProof {
            cache: ma.cache.clone(),
            compute: memo.compute_as.clone(),
            key: source[ma.key_expr.start as usize..ma.key_expr.end as usize].to_owned(),
            key_fields: key_fields
                .iter()
                .map(|fi| fields[*fi].def.name.clone())
                .collect(),
            cache_writes,
            stable_inputs,
            coupled,
            reviewed_assumptions: reviewed,
            callers: {
                let mut symbols = vec![candidate.symbol];
                symbols.extend(
                    plan_specs
                        .iter()
                        .filter(|(_, p)| p.memo_read.as_ref().is_some_and(|m| m.via == spec.export))
                        .map(|(c, _)| analysis.candidates[*c].symbol),
                );
                let nodes: Vec<Span> = std::iter::once(candidate.node)
                    .chain(
                        plan_specs
                            .iter()
                            .filter(|(_, p)| {
                                p.memo_read.as_ref().is_some_and(|m| m.via == spec.export)
                            })
                            .map(|(c, _)| analysis.candidates[*c].node),
                    )
                    .collect();
                analysis
                    .refs
                    .iter()
                    .filter(|r| {
                        r.symbol.is_some_and(|s| symbols.contains(&s))
                            && !nodes.iter().any(|n| within(r.span, *n))
                    })
                    .map(|r| line_of(&analysis.lines, r.span.start))
                    .collect()
            },
            residual: {
                let mut residual = vec![
                    "The original returns one shared cached object; the lifted function returns a fresh one. A caller that mutates the result or compares it by identity would observe the difference; the caller lines are listed but their use of the result is not analysed.".to_owned(),
                    "These checks are refusals for known patterns, not a general proof. Only an Object.freeze'd-at-every-level literal with own primitive paths is treated as stable, and only if the global Object.freeze was intact when it was created. Roots under reviewed_assumptions are the reviewer's word: direct reassignment, member writes, mutating calls, inline writing callbacks, bare uses, escaping chains and prefixes of declared paths are refused, but aliases made other ways, helpers, named callbacks, accessors and mutation outside this module are not seen.".to_owned(),
                    "The key is a template of printed numbers joined by text that cannot occur in one; the `number` annotation on each key field is trusted, and `0` and `-0` share a cache entry.".to_owned(),
                    "Inside the sampler, mentions of functions, classes and object/array literals defined in this module are refused; opaque captured bindings (DOM nodes obtained by a call, imported objects) are not analysed, and a getter or method on one of them could run module code.".to_owned(),
                    "Patching a standard builtin through an alias after the cache is filled (`const M = Math; M.min = f`, `Object.getPrototypeOf(new Map()).set = f`, `Object.defineProperty(Math, ..)`) is not detected; the original would keep serving its cached value while the lifted code recomputes with the patched builtin.".to_owned(),
                    "The proof is static; the differential replays a coherent controller (a sampled zone only changes together with its key), so it does not exercise the reset coupling.".to_owned(),
                ];
                residual.extend(sampler_side_effects);
                residual
            },
        },
        edits,
        replaced,
    })
}

/// The first member write inside `node` whose target is not a never-reassigned local that
/// was initialized with an identifier-free literal (so it may reach a parameter or captured data).
pub(super) fn first_aliasing_write(
    analysis: &Analysis,
    ref_by_start: &BTreeMap<u32, usize>,
    node: Span,
) -> Option<u32> {
    let lo = analysis
        .member_writes
        .partition_point(|s| s.start < node.start);
    analysis.member_writes[lo..]
        .iter()
        .take_while(|s| s.start < node.end)
        .find(|root| {
            let Some(&ri) = ref_by_start.get(&root.start) else {
                return true;
            };
            let r = &analysis.refs[ri];
            let safe = r.symbol.is_some_and(|sym| {
                analysis.fresh.contains(&sym)
                    && analysis
                        .scoping_symbol_spans
                        .get(&sym)
                        .is_some_and(|d| within(*d, node))
                    && !analysis
                        .refs
                        .iter()
                        .any(|w| w.symbol == Some(sym) && w.write)
            });
            !safe
        })
        .map(|root| root.start)
}
