//! Turns captured reads into declared context fields.
use super::*;
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Plan<'a> {
    pub(super) candidate: usize,
    pub(super) spec: &'a FunctionSpec,
}

/// A context field with its `from` parsed.
pub(super) struct FieldDef<'a> {
    pub(super) def: &'a ContextField,
    pub(super) root: String,
    pub(super) props: Vec<String>,
    pub(super) call: bool,
}
pub(super) fn parse_fields<'a>(context: &str, def: &'a ContextDef) -> Result<Vec<FieldDef<'a>>> {
    ensure!(
        !def.fields.is_empty(),
        "context `{context}` declares no fields"
    );
    let mut fields: Vec<FieldDef<'a>> = Vec::new();
    for field in &def.fields {
        ensure!(
            valid_identifier(&field.name),
            "`{}` is not a usable context field name",
            field.name
        );
        ensure!(
            valid_type(&field.r#type),
            "type of context field `{}` is empty or unsafe",
            field.name
        );
        let (path, call) = match field.from.strip_suffix("()") {
            Some(callee) => (callee, true),
            None => (field.from.as_str(), false),
        };
        let parts: Vec<&str> = path.split('.').collect();
        ensure!(
            parts
                .iter()
                .all(|p| !p.is_empty() && !p.starts_with(|c: char| c.is_ascii_digit()))
                && parts.iter().all(|p| {
                    p.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                })
                && (!call || parts.len() == 1),
            "`{}` is not a captured variable, a static member path or a `name()` call",
            field.from
        );
        fields.push(FieldDef {
            def: field,
            root: parts[0].to_owned(),
            props: parts[1..].iter().map(|p| (*p).to_owned()).collect(),
            call,
        });
    }
    for (i, a) in fields.iter().enumerate() {
        for b in &fields[i + 1..] {
            ensure!(
                a.def.name != b.def.name,
                "context `{context}` repeats field `{}`",
                a.def.name
            );
            ensure!(
                a.def.from != b.def.from,
                "context `{context}` repeats `{}`",
                a.def.from
            );
            if a.root == b.root {
                let n = a.props.len().min(b.props.len());
                ensure!(
                    !a.call && !b.call && !(a.props[..n] == b.props[..n]),
                    "context `{context}` fields `{}` and `{}` overlap",
                    a.def.from,
                    b.def.from
                );
            }
        }
    }
    Ok(fields)
}

pub(super) struct Edit {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) text: String,
}
/// Everything the lifter learned about one planned function's captured inputs.
#[derive(Default)]
pub(super) struct Resolution {
    pub(super) edits: Vec<Edit>,
    /// Starts of references that no longer appear in the emitted code.
    pub(super) replaced: BTreeSet<u32>,
    pub(super) reads: BTreeSet<usize>,
    pub(super) ref_field: BTreeMap<u32, usize>,
    pub(super) field_symbols: BTreeMap<usize, oxc_semantic::SymbolId>,
    pub(super) deps: BTreeSet<usize>,
    pub(super) issues: Vec<String>,
}

/// A sampler must be a guarded cache getter: `if (!U) { .. U = .. } .. return U;`.
/// Returns the cache symbol and every other captured variable the sampler writes.
pub(super) fn sampler_cache(
    analysis: &Analysis,
    sampler: usize,
) -> Result<(oxc_semantic::SymbolId, Vec<String>), String> {
    let c = &analysis.candidates[sampler];
    let Some(name) = &c.sampler else {
        return Err(
            "its body is not exactly `if (!U) { .. } return U;` with no parameters and no early return"
                .to_owned(),
        );
    };
    let outside = |s: &oxc_semantic::SymbolId| {
        analysis
            .scoping_symbol_spans
            .get(s)
            .is_none_or(|d| !within(*d, c.node))
    };
    let mut caches: BTreeSet<_> = analysis
        .refs
        .iter()
        .filter(|r| r.name == *name && within(r.span, c.node))
        .filter_map(|r| r.symbol)
        .filter(outside)
        .collect();
    let (Some(cache), true) = (caches.pop_first(), caches.is_empty()) else {
        return Err(format!("`{name}` is not one captured cache variable"));
    };
    if !analysis
        .guards
        .iter()
        .any(|(n, span)| n == name && within(*span, c.node))
    {
        return Err(format!("its fill is not guarded by `if (!{name})`"));
    }
    // It may call methods and globals, but not module functions: those could write anything.
    for call in analysis
        .calls
        .iter()
        .filter(|call| within(call.callee, c.node))
    {
        let local = analysis
            .refs
            .binary_search_by_key(&call.callee.start, |r| r.span.start)
            .ok()
            .and_then(|i| analysis.refs[i].symbol);
        if local.is_some() {
            return Err("it calls a function defined in this module, which could write any captured variable".to_owned());
        }
    }
    if analysis.class_spans.iter().any(|k| within(*k, c.node)) {
        return Err(
            "it contains a class, whose field initializers and static blocks run later".to_owned(),
        );
    }
    // No module code may run inside the sampler: any mention of a function, class or
    // object/array literal binding defined in this module (a method, getter, iterator or
    // `toString` on it could write captured state) is refused. Opaque captured bindings, such as
    // DOM nodes built by a call, are not analysed (listed in the residual).
    for r in refs_in(&analysis.refs, c.node) {
        let Some(sym) = r.symbol else { continue };
        if analysis
            .scoping_symbol_spans
            .get(&sym)
            .is_some_and(|d| within(*d, c.node))
        {
            continue;
        }
        if analysis
            .candidates
            .iter()
            .any(|k| k.symbol == sym && k.symbol != c.symbol)
            || analysis.literal_bindings.contains(&sym)
        {
            return Err(format!(
                "it mentions `{}`, a function, class or object defined in this module whose code could write captured state",
                r.name
            ));
        }
    }
    for call in analysis.any_calls.iter().filter(|k| within(k.call, c.node)) {
        match call.kind {
            2 => {
                return Err("it calls through an expression this tool cannot resolve (for example `(0, f)()`)".to_owned());
            }
            1 => {
                let rooted = call
                    .root
                    .and_then(|root| {
                        analysis
                            .refs
                            .binary_search_by_key(&root.start, |r| r.span.start)
                            .ok()
                    })
                    .and_then(|i| analysis.refs[i].symbol);
                if rooted.is_some_and(|sym| analysis.candidates.iter().any(|k| k.symbol == sym)) {
                    return Err("it calls a method of a function defined in this module (`f.call()`), which could write any captured variable".to_owned());
                }
            }
            _ => {}
        }
    }
    let others = analysis
        .refs
        .iter()
        .filter(|r| r.write && within(r.span, c.node))
        .filter(|r| r.symbol.is_some_and(|s| s != cache && outside(&s)))
        .map(|r| r.name.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok((cache, others))
}

/// A field names a variable; every reference that matches it must be the same binding.
pub(super) fn bind_field(
    out: &mut Resolution,
    field: usize,
    symbol: oxc_semantic::SymbolId,
    name: &str,
) {
    if let Some(previous) = out.field_symbols.insert(field, symbol)
        && previous != symbol
    {
        let text = format!(
            "`{name}` refers to more than one binding inside one function, so a declared input cannot stand for it"
        );
        if !out.issues.contains(&text) {
            out.issues.push(text);
        }
    }
}

pub(super) fn resolve(
    analysis: &Analysis,
    ci: usize,
    context: Option<(&ContextUse, &[FieldDef<'_>])>,
    planned: &BTreeMap<usize, Option<&str>>,
    exempt_cache: Option<&str>,
    ref_by_start: &BTreeMap<u32, usize>,
) -> Resolution {
    let candidate = &analysis.candidates[ci];
    let by_symbol: BTreeMap<_, _> = analysis
        .candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (c.symbol, i))
        .collect();
    let mut out = Resolution::default();
    let issue = |out: &mut Resolution, text: String| {
        if !out.issues.contains(&text) {
            out.issues.push(text);
        }
    };
    for r in refs_in(&analysis.refs, candidate.node) {
        let Some(symbol) = r.symbol else { continue };
        if symbol == candidate.symbol || Some(symbol) == candidate.inner_name {
            // A self-call of a context function passes the context on.
            if let Some((usage, _)) = context
                && !r.write
            {
                match analysis
                    .calls
                    .iter()
                    .find(|c| c.callee.start == r.span.start)
                {
                    Some(call) if call.spread || call.argc != candidate.params.len() => issue(
                        &mut out,
                        format!(
                            "`{}` calls itself with {} argument(s) (or a spread) but declares {}, so the context cannot be appended safely",
                            r.name,
                            call.argc,
                            candidate.params.len()
                        ),
                    ),
                    Some(call) => {
                        let (at, text) = match call.last_arg_end {
                            Some(end) => (end as usize, format!(", {}", usage.param)),
                            None => (call.call.end as usize - 1, usage.param.clone()),
                        };
                        out.edits.push(Edit {
                            start: at,
                            end: at,
                            text,
                        });
                    }
                    None => issue(
                        &mut out,
                        format!(
                            "`{}` refers to itself other than by calling itself, so the context cannot be passed on",
                            r.name
                        ),
                    ),
                }
            }
            continue;
        }
        let declared = analysis.scoping_symbol_spans.get(&symbol);
        if declared.is_some_and(|d| within(*d, candidate.node)) {
            continue;
        }
        let verb = if r.write { "writes" } else { "reads" };
        let matches: Vec<(usize, &FieldDef<'_>)> = context
            .map(|(_, fields)| {
                fields
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.root == r.name)
                    .collect()
            })
            .unwrap_or_default();
        if let Some((usage, fields)) = context
            && !matches.is_empty()
        {
            let target = |f: &FieldDef<'_>| format!("{}.{}", usage.param, f.def.name);
            if r.write {
                issue(&mut out, format!("writes captured `{}`", r.name));
                continue;
            }
            if let Some((fi, f)) = matches.iter().find(|(_, f)| f.call) {
                match analysis
                    .calls
                    .iter()
                    .find(|c| c.callee.start == r.span.start && c.argc == 0)
                {
                    Some(call) => {
                        match by_symbol.get(&symbol) {
                            Some(&sampler) => match sampler_cache(analysis, sampler) {
                                Ok((_, others)) => {
                                    for name in others {
                                        if fields.iter().any(|g| g.root == name) {
                                            issue(
                                                &mut out,
                                                format!(
                                                    "sampler `{}` writes the declared input `{name}`",
                                                    r.name
                                                ),
                                            );
                                        }
                                    }
                                }
                                Err(why) => issue(
                                    &mut out,
                                    format!(
                                        "`{}` is not a guarded cache getter, so its calls are not one input: {why}",
                                        r.name
                                    ),
                                ),
                            },
                            None => issue(
                                &mut out,
                                format!("`{}` is not a function in this module", r.name),
                            ),
                        }
                        out.edits.push(Edit {
                            start: call.call.start as usize,
                            end: call.call.end as usize,
                            text: target(f),
                        });
                        out.replaced.insert(r.span.start);
                        out.reads.insert(*fi);
                        out.ref_field.insert(r.span.start, *fi);
                        bind_field(&mut out, *fi, symbol, &r.name);
                    }
                    None => issue(
                        &mut out,
                        format!("`{}` is used other than as the call `{}()`", r.name, r.name),
                    ),
                }
                continue;
            }
            let chain = analysis
                .chains
                .get(&r.span.start)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let hit = matches.iter().find(|(_, f)| {
                f.props.len() <= chain.len()
                    && f.props
                        .iter()
                        .zip(chain)
                        .all(|(want, (have, _))| want == have)
            });
            match hit {
                Some((_, f))
                    if f.props.is_empty()
                        && !matches!(f.def.r#type.trim(), "number" | "string" | "boolean")
                        && !analysis.member_roots.contains(&r.span.start) =>
                {
                    issue(
                        &mut out,
                        format!(
                            "declared input `{}` is used as a bare value, which could be mutated by the callee or aliased",
                            r.name
                        ),
                    );
                }
                Some((fi, f)) => {
                    let end = match f.props.len() {
                        0 => r.span.end,
                        n => chain[n - 1].1,
                    };
                    out.edits.push(Edit {
                        start: r.span.start as usize,
                        end: end as usize,
                        text: target(f),
                    });
                    out.replaced.insert(r.span.start);
                    out.reads.insert(*fi);
                    out.ref_field.insert(r.span.start, *fi);
                    bind_field(&mut out, *fi, symbol, &r.name);
                }
                None => issue(
                    &mut out,
                    format!(
                        "reads captured `{}` outside the declared context fields ({})",
                        r.name,
                        fields
                            .iter()
                            .filter(|f| f.root == r.name)
                            .map(|f| f.def.from.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ),
            }
            continue;
        }
        if exempt_cache == Some(r.name.as_str()) {
            // Validated against the memoization shape by the caller.
            continue;
        }
        if let Some(&dep) = by_symbol.get(&symbol) {
            out.deps.insert(dep);
            if let Some(Some(dep_context)) = planned.get(&dep) {
                if context.is_none_or(|(usage, _)| usage.r#type != *dep_context) {
                    issue(
                        &mut out,
                        format!(
                            "calls `{}`, which needs context `{dep_context}`",
                            analysis.candidates[dep].name
                        ),
                    );
                    continue;
                }
                let param = context.map(|(u, _)| u.param.as_str()).unwrap_or_default();
                match analysis
                    .calls
                    .iter()
                    .find(|c| c.callee.start == r.span.start)
                {
                    Some(call)
                        if call.spread || call.argc != analysis.candidates[dep].params.len() =>
                    {
                        issue(
                            &mut out,
                            format!(
                                "`{}` is called with {} argument(s) (or a spread) but declares {}, so the context cannot be appended safely",
                                r.name,
                                call.argc,
                                analysis.candidates[dep].params.len()
                            ),
                        );
                    }
                    Some(call) => {
                        let (at, text) = match call.last_arg_end {
                            Some(end) => (end as usize, format!(", {param}")),
                            None => (call.call.end as usize - 1, param.to_owned()),
                        };
                        out.edits.push(Edit {
                            start: at,
                            end: at,
                            text,
                        });
                    }
                    None => issue(
                        &mut out,
                        format!(
                            "`{}` needs context but is used other than as a direct call",
                            r.name
                        ),
                    ),
                }
            }
            continue;
        }
        issue(&mut out, format!("{verb} captured `{}`", r.name));
    }
    let lo = analysis
        .member_writes
        .partition_point(|s| s.start < candidate.node.start);
    for root in analysis.member_writes[lo..]
        .iter()
        .take_while(|s| s.start < candidate.node.end)
    {
        if within(*root, candidate.node)
            && let Some(&ri) = ref_by_start.get(&root.start)
        {
            let r = &analysis.refs[ri];
            let outside = r.symbol.is_some_and(|s| {
                analysis
                    .scoping_symbol_spans
                    .get(&s)
                    .is_none_or(|d| !within(*d, candidate.node))
            });
            if outside {
                issue(&mut out, format!("writes through captured `{}`", r.name));
            } else if context.is_some()
                && !r.symbol.is_some_and(|sym| {
                    analysis.fresh.contains(&sym)
                        && !analysis
                            .refs
                            .iter()
                            .any(|w| w.symbol == Some(sym) && w.write)
                })
            {
                // A local could alias a declared input (`let q = inputs.ribs[0]; q.x = 1`).
                issue(
                    &mut out,
                    format!(
                        "writes through `{}`, which is not a local initialized with a fresh array, object or Map and never reassigned, so it may alias a captured input",
                        r.name
                    ),
                );
            }
        }
    }
    out
}

pub(super) fn closure_of(
    resolutions: &BTreeMap<usize, Resolution>,
    start: usize,
) -> BTreeSet<usize> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![start];
    while let Some(next) = stack.pop() {
        if !seen.insert(next) {
            continue;
        }
        if let Some(res) = resolutions.get(&next) {
            stack.extend(res.deps.iter().copied());
        }
    }
    seen
}
