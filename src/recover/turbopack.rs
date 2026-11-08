//! Static lowering of a deliberately narrow Turbopack client-module ABI.
//! Parsing never evaluates captured JavaScript. Unknown runtime operations fail.
use anyhow::{Context, Result, bail, ensure};
use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::{GetSpan, SourceType, Span};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Module {
    pub factory: String,
    pub source: String,
    pub ids: Vec<u64>,
}
#[derive(Default)]
pub struct Registry {
    pub modules: BTreeMap<u64, Arc<Module>>,
}
struct Registrations<'s> {
    source: &'s str,
    entries: Vec<(Vec<u64>, Span)>,
}
impl<'a> Visit<'a> for Registrations<'_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Expression::StaticMemberExpression(member) = &call.callee
            && member.property.name == "push"
            && member
                .object
                .span()
                .source_text(self.source)
                .contains("globalThis.TURBOPACK")
            && let Some(Argument::ArrayExpression(array)) = call.arguments.first()
        {
            let mut ids = Vec::new();
            for element in array.elements.iter().skip(1) {
                match element {
                    ArrayExpressionElement::NumericLiteral(n)
                        if n.value.fract() == 0.0
                            && n.value >= 0.0
                            && n.value <= u32::MAX as f64 =>
                    {
                        ids.push(n.value as u64)
                    }
                    ArrayExpressionElement::ArrowFunctionExpression(f) if !ids.is_empty() => {
                        self.entries.push((std::mem::take(&mut ids), f.span))
                    }
                    _ => ids.clear(),
                }
            }
        }
        walk::walk_call_expression(self, call);
    }
}
impl Registry {
    pub fn add(&mut self, name: &str, source: &str) -> Result<()> {
        if !source.contains("globalThis.TURBOPACK") {
            return Ok(());
        }
        let arena = Allocator::default();
        let parsed = Parser::new(&arena, source, SourceType::cjs()).parse();
        ensure!(
            parsed.diagnostics.is_empty(),
            "invalid JavaScript registration in {name}"
        );
        let mut visitor = Registrations {
            source,
            entries: Vec::new(),
        };
        visitor.visit_program(&parsed.program);
        for (ids, span) in visitor.entries {
            ensure!(
                span.size() <= 4 * 1024 * 1024,
                "client factory byte limit exceeded"
            );
            let module = Arc::new(Module {
                factory: span.source_text(source).into(),
                source: name.into(),
                ids: ids.clone(),
            });
            for id in ids {
                if let Some(previous) = self.modules.get(&id) {
                    ensure!(
                        previous.factory == module.factory && previous.ids == module.ids,
                        "conflicting client module {id}"
                    );
                } else {
                    ensure!(
                        self.modules.len() < 8192,
                        "client module registry limit exceeded"
                    );
                    self.modules.insert(id, Arc::clone(&module));
                }
            }
        }
        ensure!(
            self.modules.len() <= 8192,
            "client module registry limit exceeded"
        );
        Ok(())
    }
    pub fn bridge(&self, id: u64) -> Option<&'static str> {
        self.bridge_inner(id, &mut BTreeSet::new())
    }
    fn bridge_inner(&self, id: u64, seen: &mut BTreeSet<u64>) -> Option<&'static str> {
        if seen.len() >= 8 || !seen.insert(id) {
            return None;
        }
        let code = &self.modules.get(&id)?.factory;
        if code.contains(".renderToStaticMarkup") && code.contains(".renderToString") {
            return Some("react-dom/server.browser");
        }
        if code.contains(".useEffect=")
            && code.contains(".useState=")
            && code.contains("react.transitional.element")
        {
            return Some("react");
        }
        if code.contains(".jsx=")
            && code.contains(".jsxs=")
            && code.contains("react.transitional.element")
        {
            return Some("react/jsx-runtime");
        }
        // CommonJS forwarding modules are accepted only when their *entire* body
        // forwards one static require, subsequently identified as a known React ABI.
        let pattern = regex::Regex::new(r#"(?s)^\([^)]*\)=>\{\s*["']use strict["'];\s*[A-Za-z_$][\w$]*\.exports=[A-Za-z_$][\w$]*\.r\((\d+)\);?\s*\}$"#).ok()?;
        let target = pattern.captures(code)?.get(1)?.as_str().parse().ok()?;
        self.bridge_inner(target, seen)
    }
}

struct Lower<'s> {
    source: &'s str,
    scoping: &'s Scoping,
    runtime: Span,
    edits: Vec<(Span, String)>,
    imports: Vec<u64>,
    conditional_depth: usize,
    exports: BTreeMap<u64, BTreeSet<String>>,
    ids: &'s [u64],
    errors: Vec<String>,
}
fn conditional(kind: AstKind<'_>) -> bool {
    matches!(
        kind,
        AstKind::Function(_)
            | AstKind::ArrowFunctionExpression(_)
            | AstKind::IfStatement(_)
            | AstKind::SwitchStatement(_)
            | AstKind::ForStatement(_)
            | AstKind::ForInStatement(_)
            | AstKind::ForOfStatement(_)
            | AstKind::WhileStatement(_)
            | AstKind::DoWhileStatement(_)
            | AstKind::TryStatement(_)
            | AstKind::LogicalExpression(_)
            | AstKind::ConditionalExpression(_)
    )
}
impl Lower<'_> {
    fn exports(&mut self, call: &CallExpression<'_>) -> Result<String> {
        let Some(Argument::ArrayExpression(entries)) = call.arguments.first() else {
            bail!("nonliteral export registration")
        };
        let target = match call.arguments.get(1) {
            None if self.ids.len() == 1 => self.ids[0],
            Some(Argument::NumericLiteral(n))
                if n.value >= 0.0
                    && n.value <= u32::MAX as f64
                    && n.value.fract() == 0.0
                    && call.arguments.len() == 2 =>
            {
                n.value as u64
            }
            _ => bail!("ambiguous client export namespace"),
        };
        ensure!(
            self.ids.contains(&target),
            "export targets an unregistered client namespace"
        );
        let mut fields = Vec::new();
        let mut i = 0;
        while i < entries.elements.len() {
            let Some(ArrayExpressionElement::StringLiteral(name)) = entries.elements.get(i) else {
                bail!("nonliteral export name")
            };
            let name = name
                .value
                .as_str()
                .context("non-UTF8 export name")?
                .to_owned();
            ensure!(name != "__esModule", "reserved client export marker");
            ensure!(
                self.exports.entry(target).or_default().insert(name.clone()),
                "duplicate recovered export {name}"
            );
            let value = entries
                .elements
                .get(i + 1)
                .context("missing export value")?;
            let descriptor = if matches!(value, ArrayExpressionElement::NumericLiteral(n) if n.value == 0.0)
            {
                let value = entries
                    .elements
                    .get(i + 2)
                    .context("missing direct export value")?;
                i += 3;
                format!("value:{}", value.span().source_text(self.source))
            } else {
                ensure!(
                    matches!(value,ArrayExpressionElement::ArrowFunctionExpression(f) if f.params.items.is_empty() && f.params.rest.is_none()),
                    "unsupported export getter"
                );
                i += 2;
                format!("get:{}", value.span().source_text(self.source))
            };
            fields.push(format!(
                "[{}]:{{enumerable:true,{descriptor}}}",
                serde_json::to_string(&name)?
            ));
        }
        Ok(format!(
            "(void Object.seal(Object.defineProperties(__mochiModule{target},{{__esModule:{{value:true}},[Symbol.toStringTag]:{{value:'Module'}},{}}})))",
            fields.join(",")
        ))
    }
}
impl<'a> Visit<'a> for Lower<'_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        if conditional(kind) {
            self.conditional_depth += 1;
        }
    }
    fn leave_node(&mut self, kind: AstKind<'a>) {
        if conditional(kind) {
            self.conditional_depth -= 1;
        }
    }
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Expression::StaticMemberExpression(member) = &call.callee
            && let Expression::Identifier(object) = &member.object
            && object
                .reference_id
                .get()
                .and_then(|id| self.scoping.get_reference(id).symbol_id())
                .is_some_and(|id| self.scoping.symbol_span(id) == self.runtime)
        {
            if self.conditional_depth > 0 && matches!(member.property.name.as_str(), "i" | "s") {
                self.errors
                    .push("conditional or deferred client registration/import".into());
                return;
            }
            let edit = match member.property.name.as_str() {
                "i" => match call.arguments.as_slice() {
                    [Argument::NumericLiteral(n)]
                        if n.value >= 0.0
                            && n.value <= u32::MAX as f64
                            && n.value.fract() == 0.0 =>
                    {
                        let id = n.value as u64;
                        if !self.imports.contains(&id) {
                            self.imports.push(id);
                        }
                        Ok(format!("__mochiImport{id}"))
                    }
                    _ => Err(anyhow::anyhow!("nonliteral client import")),
                },
                "s" => self.exports(call),
                "P" => match call.arguments.as_slice() {
                    [Argument::StringLiteral(s)] => s
                        .value
                        .as_str()
                        .context("non-UTF8 provenance path")
                        .and_then(|s| serde_json::to_string(s).map_err(Into::into)),
                    _ => Err(anyhow::anyhow!("dynamic provenance path")),
                },
                method => Err(anyhow::anyhow!(
                    "unsupported client runtime operation {method}"
                )),
            };
            match edit {
                Ok(edit) => self.edits.push((call.span, edit)),
                Err(error) => self.errors.push(error.to_string()),
            }
            return;
        }
        walk::walk_call_expression(self, call);
    }
}
pub struct Lowered {
    pub body: String,
    pub imports: Vec<u64>,
    pub exports: BTreeMap<u64, BTreeSet<String>>,
}
pub fn lower(factory: &str, ids: &[u64]) -> Result<Lowered> {
    ensure!(!ids.is_empty(), "missing registered client namespace");
    let source = format!("const __mochiFactory=({factory});");
    let arena = Allocator::default();
    let parsed = Parser::new(&arena, &source, SourceType::mjs()).parse();
    ensure!(parsed.diagnostics.is_empty(), "invalid client factory");
    let Statement::VariableDeclaration(decl) = &parsed.program.body[0] else {
        bail!("missing client factory")
    };
    let Some(Expression::ParenthesizedExpression(paren)) = &decl.declarations[0].init else {
        bail!("missing parenthesized factory")
    };
    let Expression::ArrowFunctionExpression(function) = &paren.expression else {
        bail!("unsupported client factory")
    };
    ensure!(
        function.params.items.len() == 1 && function.params.rest.is_none(),
        "unsupported CommonJS client module"
    );
    let BindingPattern::BindingIdentifier(runtime) = &function.params.items[0].pattern else {
        bail!("destructured client runtime")
    };
    let ArrowFunctionBody::FunctionBody(body) = &function.body else {
        bail!("non-block client factory")
    };
    let semantic = SemanticBuilder::new_compiler().build(&parsed.program);
    ensure!(
        semantic.diagnostics.is_empty(),
        "invalid client factory bindings"
    );
    ensure!(
        !semantic
            .semantic
            .scoping()
            .symbol_names()
            .any(|name| name.starts_with("__mochiModule") || name.starts_with("__mochiImport")),
        "client source collides with reserved recovery bindings"
    );
    ensure!(
        !semantic.semantic.scoping().symbol_ids().any(|id| Some(
            semantic.semantic.scoping().symbol_scope_id(id)
        ) == function.scope_id.get()
            && matches!(
                semantic.semantic.scoping().symbol_name(id),
                "Object" | "Symbol"
            )),
        "client source shadows namespace builtins"
    );
    let mut visitor = Lower {
        source: &source,
        scoping: semantic.semantic.scoping(),
        runtime: runtime.span,
        edits: Vec::new(),
        imports: Vec::new(),
        conditional_depth: 0,
        exports: BTreeMap::new(),
        ids,
        errors: Vec::new(),
    };
    visitor.visit_function_body(body);
    ensure!(visitor.errors.is_empty(), "{}", visitor.errors.join("; "));
    let start = body.span.start as usize + 1;
    let end = body.span.end as usize - 1;
    visitor.edits.sort_by_key(|(span, _)| span.start);
    let mut result = String::new();
    let mut cursor = start;
    for (span, replacement) in visitor.edits {
        ensure!(
            span.start as usize >= cursor && span.end as usize <= end,
            "overlapping client transformations"
        );
        result.push_str(&source[cursor..span.start as usize]);
        result.push_str(&replacement);
        cursor = span.end as usize;
    }
    result.push_str(&source[cursor..end]);
    // The original runtime parameter must be completely eliminated. A semantic
    // check distinguishes unresolved runtime references from shadowed locals.
    let checked = Parser::new(&arena, &result, SourceType::mjs()).parse();
    ensure!(
        checked.diagnostics.is_empty(),
        "invalid lowered client source"
    );
    let checked_semantic = SemanticBuilder::new_compiler().build(&checked.program);
    ensure!(
        !checked_semantic
            .semantic
            .scoping()
            .root_unresolved_references()
            .contains_key(runtime.name.as_str()),
        "unlowered client runtime reference"
    );
    Ok(Lowered {
        body: result,
        imports: visitor.imports,
        exports: visitor.exports,
    })
}
pub(super) fn format_shape(program: &Program<'_>) -> Result<()> {
    #[derive(Default)]
    struct Shape {
        depth: usize,
        nodes: usize,
        too_deep: bool,
    }
    impl<'a> Visit<'a> for Shape {
        fn enter_node(&mut self, _: AstKind<'a>) {
            self.depth += 1;
            self.nodes += 1;
        }
        fn leave_node(&mut self, _: AstKind<'a>) {
            self.depth -= 1;
        }
        fn visit_expression(&mut self, expression: &Expression<'a>) {
            if self.depth >= 256 {
                self.too_deep = true;
            } else {
                walk::walk_expression(self, expression);
            }
        }
        fn visit_statement(&mut self, statement: &Statement<'a>) {
            if self.depth >= 256 {
                self.too_deep = true;
            } else {
                walk::walk_statement(self, statement);
            }
        }
    }
    let mut shape = Shape::default();
    shape.visit_program(program);
    ensure!(
        !shape.too_deep && shape.nodes <= 1_000_000,
        "client source shape budget exceeded"
    );
    Ok(())
}
pub fn format(source: &str) -> Result<String> {
    let arena = Allocator::default();
    let parsed = Parser::new(&arena, source, SourceType::mjs()).parse();
    ensure!(
        parsed.diagnostics.is_empty(),
        "generated client source is invalid"
    );
    format_shape(&parsed.program)?;
    let code = Codegen::new().build(&parsed.program).code;
    ensure!(
        code.len() <= 16 * 1024 * 1024,
        "formatted client file byte limit exceeded"
    );
    Ok(code)
}
pub fn localize(
    source: &str,
    source_type: SourceType,
    base: &url::Url,
    assets: &BTreeMap<String, String>,
) -> Result<String> {
    struct Strings<'s> {
        base: &'s url::Url,
        assets: &'s BTreeMap<String, String>,
        edits: Vec<(Span, String)>,
    }
    impl<'a> Visit<'a> for Strings<'_> {
        fn visit_string_literal(&mut self, literal: &StringLiteral<'a>) {
            if let Some(value) = literal.value.as_str()
                && let Ok(url) = self.base.join(value)
                && let Some(local) = self.assets.get(url.as_str())
            {
                // serde serialization cannot fail for a string.
                if let Ok(value) = serde_json::to_string(local) {
                    self.edits.push((literal.span, value));
                }
            }
        }
    }
    let arena = Allocator::default();
    let parsed = Parser::new(&arena, source, source_type).parse();
    ensure!(
        parsed.diagnostics.is_empty(),
        "invalid source during client asset localization"
    );
    let mut visitor = Strings {
        base,
        assets,
        edits: Vec::new(),
    };
    visitor.visit_program(&parsed.program);
    visitor.edits.sort_by_key(|(s, _)| s.start);
    let mut result = String::new();
    let mut cursor = 0;
    for (span, value) in visitor.edits {
        ensure!(
            span.start as usize >= cursor,
            "overlapping client asset literals"
        );
        result.push_str(&source[cursor..span.start as usize]);
        result.push_str(&value);
        cursor = span.end as usize;
    }
    result.push_str(&source[cursor..]);
    let parsed = Parser::new(&arena, &result, source_type).parse();
    ensure!(
        parsed.diagnostics.is_empty(),
        "invalid localized client source"
    );
    format_shape(&parsed.program)?;
    let code = Codegen::new().build(&parsed.program).code;
    ensure!(
        code.len() <= 16 * 1024 * 1024,
        "localized client file byte limit exceeded"
    );
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovers_scoped_imports_live_getters_and_alias_registrations() {
        let mut registry = Registry::default();
        registry.add("chunk.js",r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push([document.currentScript,12,13,e=>{let x=e.i(9);function nested(e){return e.i(777)}e.s(["default",()=>x,"answer",0,42],12);e.s(["other",()=>nested],13)}]);"#).unwrap();
        assert_eq!(registry.modules.len(), 2);
        let lowered = lower(&registry.modules[&12].factory, &registry.modules[&12].ids).unwrap();
        assert_eq!(lowered.imports, vec![9]);
        assert!(Arc::ptr_eq(&registry.modules[&12], &registry.modules[&13]));
        assert_eq!(lowered.exports[&13], BTreeSet::from(["other".into()]));
        assert_eq!(
            lowered.exports[&12],
            BTreeSet::from(["default".into(), "answer".into()])
        );
        assert!(
            lowered.body.contains("e.i(777)"),
            "shadowed parameters are not bundler imports"
        );
        assert!(lowered.body.contains("get:()=>x"));
        assert!(lowered.body.contains("value:42"));
        assert!(lowered.body.contains("Object.seal"));
        assert!(lowered.body.contains("__esModule:{value:true}"));
        assert!(
            format(&format!("let __mochiExports={{}};{}", lowered.body))
                .unwrap()
                .contains("function nested")
        );
        let proto = lower("e=>{e.s(['__proto__',0,42])}", &[1]).unwrap();
        assert!(proto.body.contains("[\"__proto__\"]"));
        let ids = (0..8193)
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert!(
            Registry::default()
                .add(
                    "oversized.js",
                    &format!("globalThis.TURBOPACK.push([0,{ids},e=>{{}}]);")
                )
                .is_err()
        );
    }
    #[test]
    fn refuses_unknown_abi_dynamic_imports_conflicting_ids_and_invalid_source() {
        for code in [
            "e=>{e.i(name)}",
            "e=>{e.r(4)}",
            "e=>{const value=e.g;e.s(['x',()=>value])}",
            "e=>{e.s(['x',()=>1,'x',()=>2])}",
            "e=>{function later(){return e.i(9)}}",
            "e=>{if(condition)e.i(9)}",
            "e=>{const __mochiModule1=0;e.s(['x',()=>__mochiModule1])}",
        ] {
            assert!(lower(code, &[1]).is_err(), "{code}");
        }
        assert_eq!(
            lower(
                "e=>{let a=e.i(20),b=e.i(3);e.i(20);e.s(['default',()=>a])}",
                &[1]
            )
            .unwrap()
            .imports,
            vec![20, 3]
        );
        let mut registry = Registry::default();
        registry
            .add(
                "one.js",
                "globalThis.TURBOPACK.push([0,1,e=>{e.s(['x',()=>1])}]);",
            )
            .unwrap();
        assert!(
            registry
                .add(
                    "two.js",
                    "globalThis.TURBOPACK.push([0,1,e=>{e.s(['x',()=>2])}]);"
                )
                .is_err()
        );
        assert!(
            registry
                .add("broken.js", "globalThis.TURBOPACK.push([bad")
                .is_err()
        );
    }
}
