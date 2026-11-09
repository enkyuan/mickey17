//! Parsing, scope analysis and the facts the lifter proves things from.
use super::*;
use anyhow::{Result, ensure};
use oxc_allocator::Allocator;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_semantic::{ScopeFlags, Scoping, SemanticBuilder};
use oxc_span::{GetSpan, SourceType, Span};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Candidate {
    pub(super) name: String,
    pub(super) line: usize,
    pub(super) symbol: oxc_semantic::SymbolId,
    pub(super) node: Span,
    pub(super) kind: &'static str,
    pub(super) params: Vec<(String, oxc_semantic::SymbolId)>,
    pub(super) body: Span,
    pub(super) expression: bool,
    pub(super) blockers: BTreeSet<String>,
    /// Blockers that are not captured reads or writes (`this`, unlisted
    /// globals, parameter shapes); a context cannot clear them.
    pub(super) fixed: BTreeSet<String>,
    pub(super) deps: BTreeSet<usize>,
    pub(super) closed: bool,
    pub(super) memo: Result<MemoAst, String>,
    pub(super) seq_call: Option<SeqCall>,
    /// A named function expression's own binding (`function g(){ g() }`).
    pub(super) inner_name: Option<oxc_semantic::SymbolId>,
    /// `if (!U) { .. } return U;` with no parameters: the cache `U`.
    pub(super) sampler: Option<String>,
    /// The spans of the tested and returned cache identifiers of a sampler.
    pub(super) sampler_refs: Vec<Span>,
}
/// The write-back memoization shape: `let KEY = ..; if (!C || C.key !== KEY) { ..; C = { key: KEY, .. }; } ..C.field..`
#[derive(Debug, Clone)]
pub(super) struct MemoAst {
    pub(super) cache: String,
    pub(super) key_stmt: Span,
    pub(super) key_symbol: Option<oxc_semantic::SymbolId>,
    pub(super) key_expr: Span,
    pub(super) key_pure: bool,
    /// Template-literal key: the text between expressions and each expression's span.
    pub(super) key_parts: Option<(Vec<String>, Vec<Span>)>,
    /// The guarded block contains a `return`.
    pub(super) block_returns: bool,
    pub(super) block: Span,
    pub(super) test_refs: [Span; 2],
    pub(super) assign_stmt: Span,
    pub(super) assign_target: Span,
    pub(super) assign_expr: Span,
    pub(super) assign_rhs: Span,
    /// The identifier stored as `key: KEY` in the cached object.
    pub(super) key_value_ref: Option<Span>,
    /// Names declared directly by statements of the guarded block.
    pub(super) block_decls: Vec<String>,
    pub(super) fields: Vec<String>,
    /// From the end of the guard to the closing brace of the body.
    pub(super) rest: Span,
}
/// `return F(), C.field` where `F` fills `C`.
#[derive(Debug, Clone)]
pub(super) struct SeqCall {
    pub(super) callee: Span,
    pub(super) name: String,
    pub(super) rest: Span,
}
/// Every call expression, however its callee is written.
pub(super) struct AnyCall {
    pub(super) call: Span,
    /// 0: an identifier, 1: a member, 2: anything else (`(0, f)()`, `f()()`, ...).
    pub(super) kind: u8,
    /// The root variable of a member callee (`bump` in `bump.call()`).
    pub(super) root: Option<Span>,
}
pub(super) struct CallSite {
    pub(super) callee: Span,
    pub(super) call: Span,
    pub(super) argc: usize,
    pub(super) spread: bool,
    pub(super) last_arg_end: Option<u32>,
}
pub(super) struct AssignSite {
    pub(super) target: Span,
    pub(super) whole: Span,
    pub(super) plain: bool,
    pub(super) null: bool,
    /// The assigned value is an object or array literal (always truthy).
    pub(super) literal: bool,
    /// Roots of the value expressions inside an object/array literal being assigned.
    pub(super) value_roots: Vec<Span>,
    /// The assigned value is arithmetic, a number literal or a `Math.*` result.
    pub(super) numeric: bool,
    /// Every value of a fill literal is arithmetic, a primitive literal or a bare identifier.
    pub(super) values_ok: bool,
    /// The enclosing comma expression and this assignment's position in it.
    pub(super) seq: Option<(u32, usize)>,
}
/// Longest static member path seen from one root identifier: `(property, end offset)`.
pub(super) type Chain = Vec<(String, u32)>;
pub(super) struct Ref {
    pub(super) span: Span,
    pub(super) symbol: Option<oxc_semantic::SymbolId>,
    pub(super) name: String,
    pub(super) write: bool,
}
pub(super) struct Bind {
    pub(super) span: Span,
    pub(super) symbol: oxc_semantic::SymbolId,
    pub(super) name: String,
}
pub(super) struct Collector<'s> {
    pub(super) scoping: &'s Scoping,
    pub(super) lines: &'s [usize],
    pub(super) candidates: Vec<Candidate>,
    pub(super) refs: Vec<Ref>,
    pub(super) binds: Vec<Bind>,
    pub(super) this_spans: Vec<Span>,
    pub(super) shorthand: Vec<Span>,
    pub(super) chains: BTreeMap<u32, Chain>,
    pub(super) member_roots: BTreeSet<u32>,
    pub(super) member_writes: Vec<Span>,
    pub(super) calls: Vec<CallSite>,
    pub(super) assigns: Vec<AssignSite>,
    pub(super) seq_children: BTreeMap<u32, (u32, usize)>,
    /// `if (!X) { .. }` guards: the name and the guarded block.
    pub(super) guards: Vec<(String, Span)>,
    /// Symbols whose declaration initializer is `null` or absent (`undefined`).
    pub(super) null_init: BTreeSet<oxc_semantic::SymbolId>,
    /// Symbols initialized with a fresh array, object or `new Map(..)`.
    pub(super) fresh: BTreeSet<oxc_semantic::SymbolId>,
    /// String and template literals: their line breaks and indentation are data.
    pub(super) literals: Vec<Span>,
    /// `new.target`, `import.meta`, `super`.
    pub(super) meta: Vec<(Span, &'static str)>,
    /// `.constructor`, `.__proto__`, `.prototype` and string-literal keys of those names.
    pub(super) reflective: Vec<Span>,
    /// Identifiers used directly as the callee of `new`.
    pub(super) new_callees: BTreeSet<u32>,
    /// Method calls `root.a.b(args)`: the root identifier, the method, the callback arguments.
    pub(super) method_calls: Vec<MethodCall>,
    /// Imported bindings.
    pub(super) imports: BTreeSet<oxc_semantic::SymbolId>,
    /// Members assigned a value that might alias something else: `q.a = table[0]`.
    pub(super) aliasing_assigns: Vec<Span>,
    /// Roots of member assignments whose value cannot be an alias (`q.n = 1`, `q.n += x * 2`).
    pub(super) safe_member_assigns: BTreeSet<u32>,
    /// Roots of mutating method calls (`Object.assign(x, y)` mutates `x`, not `Object`).
    pub(super) mutator_roots: BTreeSet<u32>,
    /// Literal shape of variables initialized with a deeply frozen literal.
    pub(super) frozen_shapes: BTreeMap<oxc_semantic::SymbolId, FrozenShape>,
    /// Chains passed as call/`new` arguments, spread, assigned, initialized, returned or stored
    /// in a literal: the value may be aliased and edited elsewhere.
    pub(super) escaping_uses: Vec<Span>,
    /// Variables declared with `var`.
    pub(super) var_declared: BTreeSet<oxc_semantic::SymbolId>,
    pub(super) var_kind: bool,
    /// `let z = F()`: the local and the start of the callee reference.
    pub(super) call_inits: Vec<(oxc_semantic::SymbolId, u32)>,
    /// Callee starts of calls whose result is passed on, returned, stored in a literal, ...
    pub(super) escaping_calls: BTreeSet<u32>,
    pub(super) fn_spans: Vec<Span>,
    pub(super) class_spans: Vec<Span>,
    /// Declarator initializers `(variable, initializer span)`.
    pub(super) decl_inits: Vec<(oxc_semantic::SymbolId, Span)>,
    pub(super) any_calls: Vec<AnyCall>,
    /// Targets of assignments used as the value of another assignment (`a = b = {..}`).
    pub(super) chained_targets: BTreeSet<u32>,
    /// Variables initialized with an object, array, class or function literal, and classes.
    pub(super) literal_bindings: BTreeSet<oxc_semantic::SymbolId>,
    /// Whether each variable's declarator initializer is numeric (or absent).
    pub(super) decl_numeric: BTreeMap<oxc_semantic::SymbolId, bool>,
    /// Module-level hazards found while collecting: `(offset, eval_like, message)`.
    pub(super) extra_hazards: Vec<(u32, bool, String)>,
    /// Spans of export specifiers / default expressions, and directly exported declarations.
    pub(super) export_spans: Vec<Span>,
    pub(super) exported: BTreeSet<oxc_semantic::SymbolId>,
    /// `.__proto__` and `.setPrototypeOf` members.
    pub(super) proto_members: Vec<Span>,
    /// `.freeze` members, `Object.freeze(` callees, and `.Object` members.
    pub(super) freeze_members: Vec<u32>,
    pub(super) freeze_callees: BTreeSet<u32>,
    pub(super) object_members: Vec<Span>,
}
pub(super) struct MethodCall {
    pub(super) root: Span,
    pub(super) method: String,
    /// The span covering the call's arguments (callbacks are the functions inside it).
    pub(super) args: Option<Span>,
}
pub(super) fn primitive_literal(e: &Expression<'_>) -> bool {
    match strip(e) {
        Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::BigIntLiteral(_) => true,
        Expression::TemplateLiteral(t) => t.expressions.is_empty(),
        Expression::UnaryExpression(u) => {
            matches!(
                u.operator,
                UnaryOperator::UnaryNegation | UnaryOperator::UnaryPlus
            ) && matches!(strip(&u.argument), Expression::NumericLiteral(_))
        }
        _ => false,
    }
}
/// The own data properties of a literal frozen at every level.
#[derive(Clone, Debug)]
pub(super) enum FrozenShape {
    Prim,
    Obj(BTreeMap<String, FrozenShape>),
    Arr(Vec<FrozenShape>),
}
impl FrozenShape {
    /// Whether `path` names an own data property ending in a primitive.
    pub(super) fn primitive_at(&self, path: &[String]) -> bool {
        match (self, path.split_first()) {
            (FrozenShape::Prim, None) => true,
            (FrozenShape::Obj(props), Some((first, rest))) => {
                props.get(first).is_some_and(|n| n.primitive_at(rest))
            }
            (FrozenShape::Arr(items), Some((first, rest))) => first
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i))
                .is_some_and(|n| n.primitive_at(rest)),
            _ => false,
        }
    }
}
/// `Object.freeze(<literal>)` with the unshadowed global `Object`, frozen at every level.
pub(super) fn frozen_shape(scoping: &Scoping, e: &Expression<'_>) -> Option<FrozenShape> {
    let Expression::CallExpression(c) = strip(e) else {
        return None;
    };
    let Expression::StaticMemberExpression(m) = strip(&c.callee) else {
        return None;
    };
    let Expression::Identifier(object) = strip(&m.object) else {
        return None;
    };
    if m.property.name != "freeze"
        || object.name != "Object"
        || object
            .reference_id
            .get()
            .is_none_or(|r| scoping.get_reference(r).symbol_id().is_some())
        || c.arguments.len() != 1
    {
        return None;
    }
    let leaf = |v: &Expression<'_>| {
        if primitive_literal(v) {
            Some(FrozenShape::Prim)
        } else {
            frozen_shape(scoping, v)
        }
    };
    match c.arguments[0].as_expression().map(strip) {
        Some(Expression::ObjectExpression(o)) => {
            let mut props = BTreeMap::new();
            for p in &o.properties {
                let ObjectPropertyKind::ObjectProperty(p) = p else {
                    return None;
                };
                let key = match &p.key {
                    PropertyKey::StaticIdentifier(k) => k.name.to_string(),
                    PropertyKey::StringLiteral(k) => k.value.as_str()?.to_owned(),
                    _ => return None,
                };
                if p.kind != PropertyKind::Init
                    || p.method
                    || p.computed
                    || p.shorthand
                    || key == "__proto__"
                {
                    return None;
                }
                props.insert(key, leaf(&p.value)?);
            }
            Some(FrozenShape::Obj(props))
        }
        Some(Expression::ArrayExpression(a)) => {
            let mut items = Vec::new();
            for el in &a.elements {
                items.push(leaf(el.as_expression()?)?);
            }
            Some(FrozenShape::Arr(items))
        }
        _ => None,
    }
}
/// An object or array literal with no accessors, methods, spreads or computed keys at any depth.
pub(super) fn plain_literal(e: &Expression<'_>) -> bool {
    match strip(e) {
        Expression::ObjectExpression(o) => o.properties.iter().all(|p| match p {
            ObjectPropertyKind::ObjectProperty(p) => {
                p.kind == PropertyKind::Init
                    && !p.method
                    && !p.computed
                    && !matches!(&p.key, PropertyKey::StaticIdentifier(k) if k.name == "__proto__")
                    && plain_literal_value(&p.value)
            }
            _ => false,
        }),
        Expression::ArrayExpression(a) => a.elements.iter().all(|el| match el {
            ArrayExpressionElement::SpreadElement(_) => false,
            ArrayExpressionElement::Elision(_) => true,
            other => other.as_expression().is_some_and(plain_literal_value),
        }),
        _ => false,
    }
}
/// A flat object/array literal: no accessors, methods, spreads or computed keys, and no
/// value that is itself an object, array, function or class.
pub(super) fn flat_literal(e: &Expression<'_>) -> bool {
    let flat_value = |v: &Expression<'_>| {
        !matches!(
            strip(v),
            Expression::ObjectExpression(_)
                | Expression::ArrayExpression(_)
                | Expression::FunctionExpression(_)
                | Expression::ArrowFunctionExpression(_)
                | Expression::ClassExpression(_)
        )
    };
    plain_literal(e)
        && match strip(e) {
            Expression::ObjectExpression(o) => o.properties.iter().all(|p| match p {
                ObjectPropertyKind::ObjectProperty(p) => flat_value(&p.value),
                _ => false,
            }),
            Expression::ArrayExpression(a) => a
                .elements
                .iter()
                .all(|el| el.as_expression().is_none_or(flat_value)),
            _ => false,
        }
}
/// The values of an object/array literal are arithmetic, primitive literals or bare
/// identifiers (which the caller then checks one by one).
pub(super) fn fill_values_ok(e: &Expression<'_>) -> bool {
    let ok = |v: &Expression<'_>| {
        numeric_value(v) || primitive_literal(v) || matches!(strip(v), Expression::Identifier(_))
    };
    match strip(e) {
        Expression::ObjectExpression(o) => o.properties.iter().all(|p| match p {
            ObjectPropertyKind::ObjectProperty(p) => ok(&p.value),
            _ => false,
        }),
        Expression::ArrayExpression(a) => a
            .elements
            .iter()
            .all(|el| el.as_expression().is_some_and(ok)),
        _ => false,
    }
}
/// A value that is a number computed on the spot: arithmetic, a numeric literal, or `Math.*(..)`.
pub(super) fn numeric_value(e: &Expression<'_>) -> bool {
    match strip(e) {
        Expression::BinaryExpression(_) | Expression::NumericLiteral(_) => true,
        Expression::UnaryExpression(u) => matches!(
            u.operator,
            UnaryOperator::UnaryNegation | UnaryOperator::UnaryPlus
        ),
        Expression::CallExpression(c) => {
            matches!(strip(&c.callee), Expression::StaticMemberExpression(m)
            if matches!(strip(&m.object), Expression::Identifier(o) if o.name == "Math"))
        }
        _ => false,
    }
}
pub(super) fn plain_literal_value(e: &Expression<'_>) -> bool {
    match strip(e) {
        Expression::ObjectExpression(_) | Expression::ArrayExpression(_) => plain_literal(e),
        _ => true,
    }
}
/// The root of `x`, `x.a`, `x[0].b` when the expression is such a chain (a value that may alias).
/// The expressions whose value `e` can evaluate to: through parentheses, comma expressions,
/// conditionals and logical operators.
pub(super) fn producers<'e, 'a>(e: &'e Expression<'a>) -> Vec<&'e Expression<'a>> {
    match strip(e) {
        Expression::SequenceExpression(s) => {
            s.expressions.last().map(producers).unwrap_or_default()
        }
        Expression::ConditionalExpression(c) => {
            let mut out = producers(&c.consequent);
            out.extend(producers(&c.alternate));
            out
        }
        Expression::LogicalExpression(l) => {
            let mut out = producers(&l.left);
            out.extend(producers(&l.right));
            out
        }
        other => vec![other],
    }
}
/// Roots of the non-literal values inside an object/array literal (recursively).
pub(super) fn literal_value_roots(e: &Expression<'_>, out: &mut Vec<Span>) {
    match strip(e) {
        Expression::ObjectExpression(o) => {
            for p in &o.properties {
                if let ObjectPropertyKind::ObjectProperty(p) = p {
                    literal_value_roots(&p.value, out);
                }
            }
        }
        Expression::ArrayExpression(a) => {
            for el in &a.elements {
                if let Some(e) = el.as_expression() {
                    literal_value_roots(e, out);
                }
            }
        }
        other => out.extend(escape_root(other)),
    }
}
pub(super) fn escape_root(e: &Expression<'_>) -> Option<Span> {
    match strip(e) {
        Expression::Identifier(_)
        | Expression::StaticMemberExpression(_)
        | Expression::ComputedMemberExpression(_) => root_of(e),
        _ => None,
    }
}
pub(super) fn strip<'e, 'a>(mut e: &'e Expression<'a>) -> &'e Expression<'a> {
    while let Expression::ParenthesizedExpression(p) = e {
        e = &p.expression;
    }
    e
}
pub(super) fn chain_of(e: &Expression<'_>) -> Option<(Span, Chain)> {
    match strip(e) {
        Expression::Identifier(id) => Some((id.span, Vec::new())),
        Expression::StaticMemberExpression(m) => {
            let (root, mut path) = chain_of(&m.object)?;
            path.push((m.property.name.to_string(), m.property.span.end));
            Some((root, path))
        }
        _ => None,
    }
}
pub(super) fn root_of(e: &Expression<'_>) -> Option<Span> {
    match strip(e) {
        Expression::Identifier(id) => Some(id.span),
        Expression::StaticMemberExpression(m) => root_of(&m.object),
        Expression::ComputedMemberExpression(m) => root_of(&m.object),
        // `table.at(0)[0] = 1` edits an element of `table`.
        Expression::CallExpression(c) => root_of(&c.callee),
        _ => None,
    }
}
/// True when an expression mentions no identifier at all (so it cannot hold an alias).
pub(super) struct RefFree(pub(super) bool);
impl<'a> Visit<'a> for RefFree {
    fn visit_identifier_reference(&mut self, _: &IdentifierReference<'a>) {
        self.0 = false;
    }
}
pub(super) fn ref_free(e: &Expression<'_>) -> bool {
    let mut check = RefFree(true);
    check.visit_expression(e);
    check.0
}
/// True when a statement list contains a `return` outside nested functions.
pub(super) struct HasReturn(pub(super) bool);
impl<'a> Visit<'a> for HasReturn {
    fn visit_return_statement(&mut self, _: &ReturnStatement<'a>) {
        self.0 = true;
    }
    fn visit_function(&mut self, _: &Function<'a>, _: ScopeFlags) {}
    fn visit_arrow_function_expression(&mut self, _: &ArrowFunctionExpression<'a>) {}
}
pub(super) struct ForbiddenInKey(pub(super) bool);
impl<'a> Visit<'a> for ForbiddenInKey {
    fn visit_call_expression(&mut self, _: &CallExpression<'a>) {
        self.0 = true;
    }
    fn visit_new_expression(&mut self, _: &NewExpression<'a>) {
        self.0 = true;
    }
    fn visit_assignment_expression(&mut self, _: &AssignmentExpression<'a>) {
        self.0 = true;
    }
    fn visit_update_expression(&mut self, _: &UpdateExpression<'a>) {
        self.0 = true;
    }
    fn visit_tagged_template_expression(&mut self, _: &TaggedTemplateExpression<'a>) {
        self.0 = true;
    }
    fn visit_function(&mut self, _: &Function<'a>, _: ScopeFlags) {
        self.0 = true;
    }
    fn visit_arrow_function_expression(&mut self, _: &ArrowFunctionExpression<'a>) {
        self.0 = true;
    }
    fn visit_this_expression(&mut self, _: &ThisExpression) {
        self.0 = true;
    }
    fn visit_await_expression(&mut self, _: &AwaitExpression<'a>) {
        self.0 = true;
    }
    fn visit_yield_expression(&mut self, _: &YieldExpression<'a>) {
        self.0 = true;
    }
}
pub(super) fn memo_shape(stmts: &[Statement<'_>], body: Span) -> Result<MemoAst, String> {
    const NO: &str = "body does not start with a key declaration followed by a cache guard";
    let [
        Statement::VariableDeclaration(decl),
        Statement::IfStatement(guard),
        ..,
    ] = stmts
    else {
        return Err(NO.to_owned());
    };
    let [declarator] = decl.declarations.as_slice() else {
        return Err("the key declaration declares more than one variable".to_owned());
    };
    let (BindingPattern::BindingIdentifier(key), Some(key_init)) =
        (&declarator.id, &declarator.init)
    else {
        return Err("the key declaration is not `let KEY = expression`".to_owned());
    };
    let mut forbidden = ForbiddenInKey(false);
    forbidden.visit_expression(key_init);
    if guard.alternate.is_some() {
        return Err("the cache guard has an else branch".to_owned());
    }
    let bad_test = || "guard is not `!C || C.key !== KEY`".to_owned();
    let Expression::LogicalExpression(test) = strip(&guard.test) else {
        return Err(bad_test());
    };
    if test.operator != LogicalOperator::Or {
        return Err(bad_test());
    }
    let Expression::UnaryExpression(not) = strip(&test.left) else {
        return Err(bad_test());
    };
    let (UnaryOperator::LogicalNot, Expression::Identifier(c1)) =
        (not.operator, strip(&not.argument))
    else {
        return Err(bad_test());
    };
    let Expression::BinaryExpression(ne) = strip(&test.right) else {
        return Err(bad_test());
    };
    if ne.operator != BinaryOperator::StrictInequality {
        return Err(bad_test());
    }
    let (Expression::StaticMemberExpression(member), Expression::Identifier(k2)) =
        (strip(&ne.left), strip(&ne.right))
    else {
        return Err(bad_test());
    };
    let Expression::Identifier(c2) = strip(&member.object) else {
        return Err(bad_test());
    };
    if c1.name != c2.name || member.property.name != "key" || k2.name != key.name {
        return Err(bad_test());
    }
    let Statement::BlockStatement(block) = &guard.consequent else {
        return Err("the guarded branch is not a block".to_owned());
    };
    let Some(Statement::ExpressionStatement(last)) = block.body.last() else {
        return Err("the guarded block does not end with the cache assignment".to_owned());
    };
    let Expression::AssignmentExpression(assign) = &last.expression else {
        return Err("the guarded block does not end with the cache assignment".to_owned());
    };
    let (AssignmentOperator::Assign, AssignmentTarget::AssignmentTargetIdentifier(target)) =
        (assign.operator, &assign.left)
    else {
        return Err("the cache assignment is not `C = { .. }`".to_owned());
    };
    let Expression::ObjectExpression(object) = &assign.right else {
        return Err("the cache is not assigned an object literal".to_owned());
    };
    if target.name != c1.name {
        return Err("the guard and the assignment name different caches".to_owned());
    }
    let mut fields = Vec::new();
    let mut keyed = false;
    let mut key_value_ref = None;
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(p) = property else {
            return Err("the cached object uses a spread".to_owned());
        };
        let PropertyKey::StaticIdentifier(name) = &p.key else {
            return Err("the cached object has a computed or literal key".to_owned());
        };
        if p.computed || p.method || p.kind != PropertyKind::Init {
            return Err("the cached object has a computed, method or accessor property".to_owned());
        }
        if name.name == "key" {
            keyed = matches!(strip(&p.value), Expression::Identifier(v) if v.name == key.name);
            if let Expression::Identifier(v) = strip(&p.value) {
                key_value_ref = Some(v.span);
            }
        }
        fields.push(name.name.to_string());
    }
    if !keyed {
        return Err("the cached object does not store `key: KEY`".to_owned());
    }
    let rest_ok = matches!(
        &stmts[2..],
        [Statement::ReturnStatement(r)]
            if r.argument.as_ref().is_some_and(|a| matches!(strip(a),
                Expression::StaticMemberExpression(m)
                    if matches!(strip(&m.object), Expression::Identifier(o) if o.name == c1.name)
                        && fields.iter().any(|f| *f == m.property.name.as_str())))
    );
    if !rest_ok {
        return Err("the code after the guard is not exactly `return C.field;`".to_owned());
    }
    Ok(MemoAst {
        cache: c1.name.to_string(),
        key_stmt: decl.span,
        key_symbol: key.symbol_id.get(),
        key_expr: key_init.span(),
        key_pure: !forbidden.0,
        key_parts: match strip(key_init) {
            Expression::TemplateLiteral(t) => Some((
                t.quasis
                    .iter()
                    .map(|q| {
                        q.value
                            .cooked
                            .as_ref()
                            .map(|c| c.as_str().unwrap_or("").to_owned())
                            .unwrap_or_default()
                    })
                    .collect(),
                t.expressions.iter().map(|e| e.span()).collect(),
            )),
            _ => None,
        },
        block_returns: {
            let mut found = HasReturn(false);
            for statement in &block.body {
                found.visit_statement(statement);
            }
            found.0
        },
        block: block.span,
        test_refs: [c1.span, c2.span],
        assign_stmt: last.span,
        assign_target: target.span,
        assign_expr: assign.span,
        assign_rhs: object.span,
        key_value_ref,
        block_decls: {
            let mut names = Vec::new();
            for st in &block.body {
                match st {
                    Statement::VariableDeclaration(v) => {
                        for d in &v.declarations {
                            if let BindingPattern::BindingIdentifier(id) = &d.id {
                                names.push(id.name.to_string());
                            }
                        }
                    }
                    Statement::FunctionDeclaration(f) => {
                        if let Some(id) = &f.id {
                            names.push(id.name.to_string());
                        }
                    }
                    Statement::ClassDeclaration(c) => {
                        if let Some(id) = &c.id {
                            names.push(id.name.to_string());
                        }
                    }
                    _ => {}
                }
            }
            names
        },
        fields,
        rest: Span::new(guard.span.end, body.end.saturating_sub(1)),
    })
}
pub(super) fn sampler_shape(stmts: &[Statement<'_>]) -> Option<(String, Vec<Span>)> {
    let [
        Statement::IfStatement(guard),
        Statement::ReturnStatement(ret),
    ] = stmts
    else {
        return None;
    };
    if guard.alternate.is_some() {
        return None;
    }
    let Expression::UnaryExpression(not) = strip(&guard.test) else {
        return None;
    };
    let (UnaryOperator::LogicalNot, Expression::Identifier(tested)) =
        (not.operator, strip(&not.argument))
    else {
        return None;
    };
    let Expression::Identifier(returned) = strip(ret.argument.as_ref()?) else {
        return None;
    };
    let mut early = HasReturn(false);
    early.visit_statement(&guard.consequent);
    (tested.name == returned.name && !early.0)
        .then(|| (tested.name.to_string(), vec![tested.span, returned.span]))
}
pub(super) fn seq_call(stmts: &[Statement<'_>]) -> Option<SeqCall> {
    let [Statement::ReturnStatement(ret)] = stmts else {
        return None;
    };
    let Expression::SequenceExpression(seq) = strip(ret.argument.as_ref()?) else {
        return None;
    };
    let [first, last] = seq.expressions.as_slice() else {
        return None;
    };
    // `return F(), C.field;`: the cached value goes straight to the caller, to no helper.
    let Expression::StaticMemberExpression(field) = strip(last) else {
        return None;
    };
    if !matches!(strip(&field.object), Expression::Identifier(_)) {
        return None;
    }
    let Expression::CallExpression(call) = strip(first) else {
        return None;
    };
    let Expression::Identifier(callee) = strip(&call.callee) else {
        return None;
    };
    call.arguments.is_empty().then(|| SeqCall {
        callee: callee.span,
        name: callee.name.to_string(),
        rest: Span::new(first.span().end, last.span().end),
    })
}
pub(super) fn line_of(lines: &[usize], offset: u32) -> usize {
    lines.partition_point(|start| *start <= offset as usize)
}
pub(super) struct Shape {
    pub(super) node: Span,
    pub(super) kind: &'static str,
    pub(super) body: Span,
    pub(super) expression: bool,
    /// `async` or generator, which this pass does not lift.
    pub(super) modifiers: bool,
    pub(super) inner: Option<oxc_semantic::SymbolId>,
}
impl Collector<'_> {
    fn note_escape(&mut self, e: &Expression<'_>) {
        self.note_escape_with(e, false);
    }
    /// `keep_call_alias`: the value goes straight into a variable (tracked through `call_inits`),
    /// so a call result is not counted as escaping.
    fn note_escape_with(&mut self, e: &Expression<'_>, keep_call_alias: bool) {
        for p in producers(e) {
            if let Some(root) = escape_root(p) {
                self.escaping_uses.push(root);
            }
            if !keep_call_alias
                && let Expression::CallExpression(c) = p
                && let Expression::Identifier(callee) = strip(&c.callee)
            {
                self.escaping_calls.insert(callee.span.start);
            }
        }
    }
    fn member_call(&mut self, callee: &Expression<'_>, args: Option<Span>) {
        match strip(callee) {
            Expression::StaticMemberExpression(m) => {
                if let Some(root) = root_of(&m.object) {
                    if MUTATORS.contains(&m.property.name.as_str()) {
                        self.member_writes.push(root);
                        self.mutator_roots.insert(root.start);
                    }
                    self.method_calls.push(MethodCall {
                        root,
                        method: m.property.name.to_string(),
                        args,
                    });
                }
            }
            // `t["push"](x)` and `t[name](x)`: a computed method may be any mutator.
            Expression::ComputedMemberExpression(m) => {
                let mutator = match &m.expression {
                    Expression::StringLiteral(k) => {
                        k.value.as_str().is_some_and(|v| MUTATORS.contains(&v))
                    }
                    _ => true,
                };
                if mutator && let Some(root) = root_of(&m.object) {
                    self.member_writes.push(root);
                }
            }
            _ => {}
        }
    }
    fn function_like(
        &mut self,
        id: &BindingIdentifier<'_>,
        params: &FormalParameters<'_>,
        shape: Shape,
        statements: &[Statement<'_>],
    ) {
        let Shape {
            node,
            kind,
            body,
            expression,
            modifiers,
            inner,
        } = shape;
        let Some(symbol) = id.symbol_id.get() else {
            return;
        };
        let mut blockers = BTreeSet::new();
        let mut names = Vec::new();
        let sampler = if params.items.is_empty() && params.rest.is_none() {
            sampler_shape(statements)
        } else {
            None
        };
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
            fixed: blockers.clone(),
            blockers,
            deps: BTreeSet::new(),
            closed: false,
            memo: memo_shape(statements, body),
            seq_call: seq_call(statements),
            inner_name: inner,
            sampler: sampler.as_ref().map(|(n, _)| n.clone()),
            sampler_refs: sampler.map(|(_, r)| r).unwrap_or_default(),
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
    fn visit_tagged_template_expression(&mut self, it: &TaggedTemplateExpression<'a>) {
        // x.reverse`` calls the method just like x.reverse().
        self.member_call(&it.tag, None);
        self.any_calls.push(AnyCall {
            call: it.span,
            kind: 2,
            root: None,
        });
        walk::walk_tagged_template_expression(self, it);
    }
    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        if let Expression::Identifier(id) = strip(&it.callee) {
            self.new_callees.insert(id.span.start);
        }
        let global = matches!(strip(&it.callee), Expression::Identifier(id)
            if id.reference_id.get().is_some_and(|r| self.scoping.get_reference(r).symbol_id().is_none()));
        if !global {
            self.any_calls.push(AnyCall {
                call: it.span,
                kind: 2,
                root: None,
            });
        }
        for a in &it.arguments {
            if let Some(e) = a.as_expression() {
                self.note_escape(e);
            }
        }
        walk::walk_new_expression(self, it);
    }
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        for specifier in it.specifiers.iter().flatten() {
            if let Some(symbol) = specifier.local().symbol_id.get() {
                self.imports.insert(symbol);
            }
        }
        walk::walk_import_declaration(self, it);
    }
    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        self.literals.push(it.span);
    }
    fn visit_template_literal(&mut self, it: &TemplateLiteral<'a>) {
        self.literals.push(it.span);
        walk::walk_template_literal(self, it);
    }
    fn visit_super(&mut self, it: &Super) {
        self.meta.push((it.span, "super"));
    }
    fn visit_import_meta(&mut self, it: &ImportMeta) {
        self.meta.push((it.span, "`import.meta`"));
    }
    fn visit_new_target(&mut self, it: &NewTarget) {
        self.meta.push((it.span, "`new.target`"));
    }
    fn visit_assignment_target_property_identifier(
        &mut self,
        it: &AssignmentTargetPropertyIdentifier<'a>,
    ) {
        self.shorthand.push(it.span);
        walk::walk_assignment_target_property_identifier(self, it);
    }
    fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'a>) {
        if it.property.name == "constructor"
            && matches!(strip(&it.object), Expression::StaticMemberExpression(m) if m.property.name == "constructor")
        {
            self.extra_hazards.push((
                it.span.start,
                true,
                "the module reaches the `Function` constructor through `.constructor.constructor`"
                    .to_owned(),
            ));
        }
        match it.property.name.as_str() {
            "__proto__" | "setPrototypeOf" => self.proto_members.push(it.span),
            "freeze" => self.freeze_members.push(it.span.start),
            "Object" => self.object_members.push(it.span),
            _ => {}
        }
        if matches!(
            it.property.name.as_str(),
            "constructor" | "__proto__" | "prototype"
        ) {
            self.reflective.push(it.span);
        }
        if let Some((root, mut path)) = chain_of(&it.object) {
            path.push((it.property.name.to_string(), it.property.span.end));
            self.member_roots.insert(root.start);
            let longest = self.chains.entry(root.start).or_default();
            if path.len() > longest.len() {
                *longest = path;
            }
        } else if let Some(root) = root_of(&it.object) {
            self.member_roots.insert(root.start);
        }
        walk::walk_static_member_expression(self, it);
    }
    fn visit_computed_member_expression(&mut self, it: &ComputedMemberExpression<'a>) {
        if let Expression::StringLiteral(k) = &it.expression
            && let Some(key) = k.value.as_str()
        {
            match key {
                "Object" | "freeze" | "Reflect" | "__proto__" | "setPrototypeOf" => {
                    self.extra_hazards.push((
                        it.span.start,
                        false,
                        format!("the module reaches `{key}` through a computed member"),
                    ));
                }
                "constructor" | "Function" => self.extra_hazards.push((
                    it.span.start,
                    true,
                    format!("the module reaches `{key}` through a computed member"),
                )),
                _ => {}
            }
        }
        if matches!(&it.expression, Expression::StringLiteral(k) if matches!(k.value.as_str(), Some("constructor" | "__proto__" | "prototype")))
        {
            self.reflective.push(it.span);
        }
        if let Some(root) = root_of(&it.object) {
            self.member_roots.insert(root.start);
        }
        walk::walk_computed_member_expression(self, it);
    }
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let pure = matches!(strip(&it.callee), Expression::StaticMemberExpression(m)
            if matches!(strip(&m.object), Expression::Identifier(o) if matches!(o.name.as_str(), "Math" | "Number")));
        if !pure {
            for a in &it.arguments {
                if let Some(e) = a.as_expression() {
                    self.note_escape(e);
                }
            }
        }
        if let Expression::StaticMemberExpression(m) = strip(&it.callee)
            && m.property.name == "freeze"
        {
            self.freeze_callees.insert(m.span.start);
        }
        let args = it
            .arguments
            .first()
            .zip(it.arguments.last())
            .map(|(f, l)| Span::new(f.span().start, l.span().end));
        let kind = match strip(&it.callee) {
            Expression::Identifier(id) => {
                self.calls.push(CallSite {
                    callee: id.span,
                    call: it.span,
                    argc: it.arguments.len(),
                    spread: it.arguments.iter().any(|a| a.is_spread()),
                    last_arg_end: it.arguments.last().map(|a| a.span().end),
                });
                0
            }
            Expression::StaticMemberExpression(_) | Expression::ComputedMemberExpression(_) => 1,
            _ => 2,
        };
        self.any_calls.push(AnyCall {
            call: it.span,
            kind,
            root: match strip(&it.callee) {
                Expression::StaticMemberExpression(m) => root_of(&m.object),
                Expression::ComputedMemberExpression(m) => root_of(&m.object),
                _ => None,
            },
        });
        self.member_call(&it.callee, args);
        walk::walk_call_expression(self, it);
    }
    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        if let Expression::AssignmentExpression(inner) = strip(&it.right)
            && let AssignmentTarget::AssignmentTargetIdentifier(id) = &inner.left
        {
            self.chained_targets.insert(id.span.start);
        }
        let ident_target = matches!(&it.left, AssignmentTarget::AssignmentTargetIdentifier(_));
        self.note_escape_with(&it.right, ident_target);
        if let AssignmentTarget::AssignmentTargetIdentifier(id) = &it.left
            && let Some(symbol) = id
                .reference_id
                .get()
                .and_then(|r| self.scoping.get_reference(r).symbol_id())
        {
            for p in producers(&it.right) {
                if let Expression::CallExpression(c) = p
                    && let Expression::Identifier(callee) = strip(&c.callee)
                {
                    self.call_inits.push((symbol, callee.span.start));
                }
            }
        }
        let member_target = match &it.left {
            AssignmentTarget::StaticMemberExpression(m) => root_of(&m.object),
            AssignmentTarget::ComputedMemberExpression(m) => root_of(&m.object),
            _ => None,
        };
        if let Some(root) = member_target
            && (ref_free(&it.right)
                || matches!(
                    strip(&it.right),
                    Expression::BinaryExpression(_)
                        | Expression::NumericLiteral(_)
                        | Expression::UnaryExpression(_)
                ))
        {
            self.safe_member_assigns.insert(root.start);
        }
        if let AssignmentTarget::AssignmentTargetIdentifier(id) = &it.left {
            self.assigns.push(AssignSite {
                target: id.span,
                whole: it.span,
                plain: it.operator == AssignmentOperator::Assign,
                null: matches!(strip(&it.right), Expression::NullLiteral(_)),
                literal: flat_literal(&it.right),
                numeric: numeric_value(&it.right),
                values_ok: fill_values_ok(&it.right),
                value_roots: {
                    let mut roots = Vec::new();
                    literal_value_roots(&it.right, &mut roots);
                    roots
                },
                seq: self.seq_children.get(&it.span.start).copied(),
            });
        }
        walk::walk_assignment_expression(self, it);
    }
    fn visit_simple_assignment_target(&mut self, it: &SimpleAssignmentTarget<'a>) {
        let root = match it {
            SimpleAssignmentTarget::StaticMemberExpression(m) => root_of(&m.object),
            SimpleAssignmentTarget::ComputedMemberExpression(m) => root_of(&m.object),
            _ => None,
        };
        if let Some(root) = root
            && !self.safe_member_assigns.contains(&root.start)
        {
            // Destructuring and `for (q.a of ..)` targets, and unsafe values: possible alias.
            self.aliasing_assigns.push(root);
        }
        self.member_writes.extend(root);
        walk::walk_simple_assignment_target(self, it);
    }
    fn visit_unary_expression(&mut self, it: &UnaryExpression<'a>) {
        if it.operator == UnaryOperator::Delete {
            let root = match strip(&it.argument) {
                Expression::StaticMemberExpression(m) => root_of(&m.object),
                Expression::ComputedMemberExpression(m) => root_of(&m.object),
                _ => None,
            };
            self.member_writes.extend(root);
        }
        walk::walk_unary_expression(self, it);
    }
    fn visit_if_statement(&mut self, it: &IfStatement<'a>) {
        if let Expression::UnaryExpression(not) = strip(&it.test)
            && not.operator == UnaryOperator::LogicalNot
            && let Expression::Identifier(id) = strip(&not.argument)
        {
            self.guards
                .push((id.name.to_string(), it.consequent.span()));
        }
        walk::walk_if_statement(self, it);
    }
    fn visit_sequence_expression(&mut self, it: &SequenceExpression<'a>) {
        for (i, e) in it.expressions.iter().enumerate() {
            if let Expression::AssignmentExpression(a) = strip(e) {
                self.seq_children.insert(a.span.start, (it.span.start, i));
            }
        }
        walk::walk_sequence_expression(self, it);
    }
    fn visit_object_property(&mut self, p: &ObjectProperty<'a>) {
        self.note_escape(&p.value);
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
    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.fn_spans.push(it.span);
        walk::walk_arrow_function_expression(self, it);
    }
    fn visit_function(&mut self, f: &Function<'a>, flags: ScopeFlags) {
        self.fn_spans.push(f.span);
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
                    inner: None,
                },
                &body.statements,
            );
        }
        walk::walk_function(self, f, flags);
    }
    fn visit_class(&mut self, it: &Class<'a>) {
        self.fn_spans.push(it.span);
        self.class_spans.push(it.span);
        if let Some(symbol) = it.id.as_ref().and_then(|i| i.symbol_id.get()) {
            self.literal_bindings.insert(symbol);
        }
        walk::walk_class(self, it);
    }
    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        let previous = self.var_kind;
        self.var_kind = it.kind == VariableDeclarationKind::Var;
        walk::walk_variable_declaration(self, it);
        self.var_kind = previous;
    }
    fn visit_export_declaration(&mut self, it: &ExportDeclaration<'a>) {
        match &it.declaration {
            Declaration::VariableDeclaration(v) => {
                for d in &v.declarations {
                    if let BindingPattern::BindingIdentifier(id) = &d.id
                        && let Some(symbol) = id.symbol_id.get()
                    {
                        self.exported.insert(symbol);
                    }
                }
            }
            Declaration::FunctionDeclaration(f) => {
                if let Some(symbol) = f.id.as_ref().and_then(|i| i.symbol_id.get()) {
                    self.exported.insert(symbol);
                }
            }
            Declaration::ClassDeclaration(c) => {
                if let Some(symbol) = c.id.as_ref().and_then(|i| i.symbol_id.get()) {
                    self.exported.insert(symbol);
                }
            }
            _ => {}
        }
        walk::walk_export_declaration(self, it);
    }
    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        self.export_spans.push(it.span);
        walk::walk_export_named_declaration(self, it);
    }
    fn visit_export_default_declaration(&mut self, it: &ExportDefaultDeclaration<'a>) {
        self.export_spans.push(it.span);
        walk::walk_export_default_declaration(self, it);
    }
    fn visit_spread_element(&mut self, it: &SpreadElement<'a>) {
        self.note_escape(&it.argument);
        walk::walk_spread_element(self, it);
    }
    fn visit_return_statement(&mut self, it: &ReturnStatement<'a>) {
        if let Some(a) = &it.argument {
            self.note_escape(a);
        }
        walk::walk_return_statement(self, it);
    }
    fn visit_array_expression(&mut self, it: &ArrayExpression<'a>) {
        for el in &it.elements {
            if let Some(e) = el.as_expression() {
                self.note_escape(e);
            }
        }
        walk::walk_array_expression(self, it);
    }
    fn visit_variable_declarator(&mut self, d: &VariableDeclarator<'a>) {
        if let Some(init) = &d.init {
            self.note_escape_with(init, true);
        }
        if let BindingPattern::BindingIdentifier(id) = &d.id
            && let Some(symbol) = id.symbol_id.get()
        {
            if self.var_kind {
                self.var_declared.insert(symbol);
            }
            self.decl_numeric
                .insert(symbol, d.init.as_ref().is_none_or(numeric_value));
            if let Some(init) = &d.init {
                self.decl_inits.push((symbol, init.span()));
            }
            if let Some(init) = &d.init
                && matches!(
                    strip(init),
                    Expression::ObjectExpression(_)
                        | Expression::ArrayExpression(_)
                        | Expression::ClassExpression(_)
                        | Expression::FunctionExpression(_)
                        | Expression::ArrowFunctionExpression(_)
                        | Expression::NewExpression(_)
                )
            {
                self.literal_bindings.insert(symbol);
            }
            for p in d.init.as_ref().map(producers).unwrap_or_default() {
                if let Expression::CallExpression(c) = p
                    && let Expression::Identifier(callee) = strip(&c.callee)
                {
                    self.call_inits.push((symbol, callee.span.start));
                }
            }
            if let Some(shape) = d.init.as_ref().and_then(|i| frozen_shape(self.scoping, i)) {
                self.frozen_shapes.insert(symbol, shape);
            }
            match d.init.as_ref().map(strip) {
                None | Some(Expression::NullLiteral(_)) => {
                    self.null_init.insert(symbol);
                }
                Some(init @ (Expression::ArrayExpression(_) | Expression::ObjectExpression(_))) => {
                    if ref_free(init) {
                        self.fresh.insert(symbol);
                    }
                }
                Some(init @ Expression::NewExpression(n))
                    if matches!(strip(&n.callee), Expression::Identifier(c) if matches!(c.name.as_str(), "Map" | "Array")
                            && c.reference_id.get().is_some_and(|r| self.scoping.get_reference(r).symbol_id().is_none()))
                        && (n.arguments.is_empty() || ref_free(init)) =>
                {
                    self.fresh.insert(symbol);
                }
                _ => {}
            }
        }
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
                            inner: None,
                        },
                        match &f.body {
                            ArrowFunctionBody::FunctionBody(b) => &b.statements,
                            _ => &[],
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
                                inner: f.id.as_ref().and_then(|i| i.symbol_id.get()),
                            },
                            &body.statements,
                        );
                    }
                }
                _ => {}
            }
        }
        walk::walk_variable_declarator(self, d);
    }
}
/// The references inside `node`; `refs` must be sorted by start offset.
pub(super) fn refs_in(refs: &[Ref], node: Span) -> impl Iterator<Item = &Ref> {
    let lo = refs.partition_point(|r| r.span.start < node.start);
    refs[lo..]
        .iter()
        .take_while(move |r| r.span.start < node.end)
        .filter(move |r| within(r.span, node))
}
/// Whether any of the spans, sorted by start offset, lies inside `node`.
pub(super) fn any_in(sorted: &[Span], node: Span) -> bool {
    let lo = sorted.partition_point(|s| s.start < node.start);
    sorted[lo..]
        .iter()
        .take_while(|s| s.start < node.end)
        .any(|s| within(*s, node))
}
pub(super) fn within(inner: Span, outer: Span) -> bool {
    inner.start >= outer.start && inner.end <= outer.end
}

pub(super) struct Analysis {
    pub(super) candidates: Vec<Candidate>,
    pub(super) refs: Vec<Ref>,
    pub(super) binds: Vec<Bind>,
    pub(super) shorthand: Vec<Span>,
    pub(super) chains: BTreeMap<u32, Chain>,
    pub(super) member_roots: BTreeSet<u32>,
    pub(super) member_writes: Vec<Span>,
    pub(super) calls: Vec<CallSite>,
    pub(super) assigns: Vec<AssignSite>,
    pub(super) null_init: BTreeSet<oxc_semantic::SymbolId>,
    pub(super) fresh: BTreeSet<oxc_semantic::SymbolId>,
    pub(super) literals: Vec<Span>,
    pub(super) guards: Vec<(String, Span)>,
    pub(super) method_calls: Vec<MethodCall>,
    pub(super) imports: BTreeSet<oxc_semantic::SymbolId>,
    /// Variables declared more than once (`var x; var x;`).
    pub(super) redeclared: BTreeSet<oxc_semantic::SymbolId>,
    pub(super) mutator_roots: BTreeSet<u32>,
    pub(super) frozen_shapes: BTreeMap<oxc_semantic::SymbolId, FrozenShape>,
    pub(super) escaping_uses: Vec<Span>,
    pub(super) var_declared: BTreeSet<oxc_semantic::SymbolId>,
    pub(super) call_inits: Vec<(oxc_semantic::SymbolId, u32)>,
    pub(super) escaping_calls: BTreeSet<u32>,
    pub(super) fn_spans: Vec<Span>,
    pub(super) class_spans: Vec<Span>,
    pub(super) any_calls: Vec<AnyCall>,
    pub(super) chained_targets: BTreeSet<u32>,
    pub(super) literal_bindings: BTreeSet<oxc_semantic::SymbolId>,
    pub(super) decl_numeric: BTreeMap<oxc_semantic::SymbolId, bool>,
    /// Variables exported from the module, by declaration or specifier.
    pub(super) exported: BTreeSet<oxc_semantic::SymbolId>,
    /// Module-wide hazards that undermine frozen/stable proofs.
    /// `(offset, eval_like, message)`; `eval_like` hazards matter to every context lift.
    pub(super) hazards: Vec<(u32, bool, String)>,
    pub(super) lines: Vec<usize>,
    pub(super) scoping_symbol_spans: BTreeMap<oxc_semantic::SymbolId, Span>,
}
pub(super) fn analyze(source: &str) -> Result<Analysis> {
    ensure!(
        !source.contains(KEEP_BREAK),
        "the source contains U+E000, which the lifter uses internally to protect line breaks inside literals"
    );
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
        chains: BTreeMap::new(),
        member_roots: BTreeSet::new(),
        member_writes: Vec::new(),
        calls: Vec::new(),
        assigns: Vec::new(),
        seq_children: BTreeMap::new(),
        guards: Vec::new(),
        null_init: BTreeSet::new(),
        fresh: BTreeSet::new(),
        new_callees: BTreeSet::new(),
        method_calls: Vec::new(),
        imports: BTreeSet::new(),
        aliasing_assigns: Vec::new(),
        safe_member_assigns: BTreeSet::new(),
        mutator_roots: BTreeSet::new(),
        frozen_shapes: BTreeMap::new(),
        escaping_uses: Vec::new(),
        var_declared: BTreeSet::new(),
        var_kind: false,
        call_inits: Vec::new(),
        escaping_calls: BTreeSet::new(),
        fn_spans: Vec::new(),
        class_spans: Vec::new(),
        decl_inits: Vec::new(),
        any_calls: Vec::new(),
        chained_targets: BTreeSet::new(),
        literal_bindings: BTreeSet::new(),
        decl_numeric: BTreeMap::new(),
        extra_hazards: Vec::new(),
        export_spans: Vec::new(),
        exported: BTreeSet::new(),
        proto_members: Vec::new(),
        freeze_members: Vec::new(),
        freeze_callees: BTreeSet::new(),
        object_members: Vec::new(),
        literals: Vec::new(),
        meta: Vec::new(),
        reflective: Vec::new(),
    };
    collector.visit_program(&parsed.program);
    let Collector {
        mut candidates,
        refs,
        binds,
        this_spans,
        shorthand,
        chains,
        member_roots,
        member_writes,
        calls,
        assigns,
        null_init,
        fresh,
        literals,
        meta,
        reflective,
        guards,
        new_callees,
        method_calls,
        imports,
        aliasing_assigns,
        mutator_roots,
        frozen_shapes,
        escaping_uses,
        var_declared,
        call_inits,
        escaping_calls,
        fn_spans,
        class_spans,
        decl_inits,
        any_calls,
        chained_targets,
        literal_bindings,
        decl_numeric,
        extra_hazards,
        export_spans,
        exported,
        proto_members,
        freeze_members,
        freeze_callees,
        object_members,
        ..
    } = collector;
    let mut refs = refs;
    let mut binds = binds;
    let mut this_spans = this_spans;
    let mut meta = meta;
    let mut reflective = reflective;
    let mut member_writes = member_writes;
    refs.sort_by_key(|r| r.span.start);
    let mut fn_spans = fn_spans;
    fn_spans.sort_by_key(|s| s.start);
    binds.sort_by_key(|b| b.span.start);
    this_spans.sort_by_key(|s| s.start);
    meta.sort_by_key(|(s, _)| s.start);
    reflective.sort_by_key(|s| s.start);
    member_writes.sort_by_key(|s| s.start);
    let meta_spans: Vec<Span> = meta.iter().map(|(s, _)| *s).collect();
    let callee_starts: BTreeSet<u32> = calls.iter().map(|c| c.callee.start).collect();
    let first_write: BTreeMap<oxc_semantic::SymbolId, u32> = {
        let mut map = BTreeMap::new();
        for r in refs.iter().filter(|r| r.write) {
            if let Some(symbol) = r.symbol {
                map.entry(symbol).or_insert(r.span.start);
            }
        }
        map
    };
    let by_symbol: BTreeMap<_, _> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (c.symbol, i))
        .collect();
    let mut exported = exported;
    for r in &refs {
        if r.symbol.is_some() && export_spans.iter().any(|s| within(r.span, *s)) {
            exported.extend(r.symbol);
        }
    }
    let hazards: Vec<(u32, bool, String)> = {
        let mut out = Vec::new();
        for r in refs.iter().filter(|r| r.symbol.is_none()) {
            match r.name.as_str() {
                "eval" | "Function" => out.push((
                    r.span.start,
                    true,
                    format!("the module uses the global `{}`", r.name),
                )),
                "Reflect" => {
                    out.push((r.span.start, false, "the module uses `Reflect`".to_owned()))
                }
                "globalThis" => out.push((
                    r.span.start,
                    false,
                    "the module uses `globalThis`".to_owned(),
                )),
                "Object"
                    if !member_roots.contains(&r.span.start)
                        && !callee_starts.contains(&r.span.start)
                        && !new_callees.contains(&r.span.start) =>
                {
                    out.push((
                        r.span.start,
                        false,
                        "the module passes `Object` around as a value".to_owned(),
                    ));
                }
                _ => {}
            }
        }
        out.extend(extra_hazards);
        for s in &proto_members {
            out.push((
                s.start,
                false,
                "the module uses `__proto__` or `setPrototypeOf`".to_owned(),
            ));
        }
        for at in freeze_members
            .iter()
            .filter(|a| !freeze_callees.contains(a))
        {
            out.push((
                *at,
                false,
                "the module uses `.freeze` other than as the callee `Object.freeze(`".to_owned(),
            ));
        }
        for s in &object_members {
            out.push((
                s.start,
                false,
                "the module reaches `Object` through a member (for example `globalThis.Object`)"
                    .to_owned(),
            ));
        }
        out
    };
    let redeclared: BTreeSet<oxc_semantic::SymbolId> = refs
        .iter()
        .filter_map(|r| r.symbol)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|s| !scoping.symbol_redeclarations(*s).is_empty())
        .collect();
    // A binding initialized from, or assigned, something that mentions a module function or a
    // module object is module code too (`const g = bump; const util = Object.freeze({ f: bump })`).
    let mut literal_bindings = literal_bindings;
    loop {
        let mut grew = false;
        let mentions = |span: Span, set: &BTreeSet<oxc_semantic::SymbolId>| {
            refs_in(&refs, span).any(|r| {
                r.symbol
                    .is_some_and(|s| set.contains(&s) || by_symbol.contains_key(&s))
            })
        };
        for (sym, init) in &decl_inits {
            if !literal_bindings.contains(sym) && mentions(*init, &literal_bindings) {
                literal_bindings.insert(*sym);
                grew = true;
            }
        }
        for a in &assigns {
            let Some(sym) = refs
                .binary_search_by_key(&a.target.start, |r| r.span.start)
                .ok()
                .and_then(|i| refs[i].symbol)
            else {
                continue;
            };
            let rhs = Span::new(a.target.end, a.whole.end);
            if !literal_bindings.contains(&sym) && mentions(rhs, &literal_bindings) {
                literal_bindings.insert(sym);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let mut fresh = fresh;
    fresh.retain(|s| !redeclared.contains(s));
    for root in &aliasing_assigns {
        // `q.a = table[0]` makes everything reached through `q` a possible alias.
        if let Some(symbol) = refs
            .binary_search_by_key(&root.start, |r| r.span.start)
            .ok()
            .and_then(|i| refs[i].symbol)
        {
            fresh.remove(&symbol);
        }
    }
    let mut spans = BTreeMap::new();
    let ref_start: BTreeMap<u32, usize> = refs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.span.start, i))
        .collect();
    // A binding that is reassigned or declared again is not one stable function.
    let unstable = |symbol: oxc_semantic::SymbolId| -> Option<String> {
        if let Some(at) = first_write.get(&symbol) {
            return Some(format!("is reassigned at line {}", line_of(&lines, *at)));
        }
        (!scoping.symbol_redeclarations(symbol).is_empty())
            .then(|| "is declared more than once".to_owned())
    };
    for candidate in &mut candidates {
        let mut fixed: BTreeSet<String> = BTreeSet::new();
        let mut blockers: BTreeSet<String> = BTreeSet::new();
        let mut deps: BTreeSet<usize> = BTreeSet::new();
        for r in refs_in(&refs, candidate.node) {
            match r.symbol {
                None if r.write => {
                    fixed.insert(format!("writes global `{}`", r.name));
                }
                None if ALLOWED_GLOBALS.contains(&r.name.as_str()) => {
                    let member = chains
                        .get(&r.span.start)
                        .and_then(|c| c.first())
                        .map(|(p, _)| p.as_str());
                    let called = callee_starts.contains(&r.span.start)
                        || new_callees.contains(&r.span.start);
                    let allowed = match r.name.as_str() {
                        "Infinity" | "NaN" | "undefined" => true,
                        "isFinite" | "isNaN" => called,
                        "Math" => member.is_some_and(|m| MATH_MEMBERS.contains(&m)),
                        "Number" => called || member.is_some_and(|m| NUMBER_MEMBERS.contains(&m)),
                        "Array" => called || member.is_some_and(|m| ARRAY_MEMBERS.contains(&m)),
                        "Map" => called,
                        _ => false,
                    };
                    if !allowed {
                        fixed.insert(format!(
                            "uses `{}{}`, which is not on the pure allowlist of the standard globals",
                            r.name,
                            member.map(|m| format!(".{m}")).unwrap_or_default()
                        ));
                    }
                }
                None => {
                    fixed.insert(format!("global `{}`", r.name));
                }
                Some(symbol)
                    if symbol == candidate.symbol || Some(symbol) == candidate.inner_name =>
                {
                    if r.write {
                        fixed.insert(format!("writes captured `{}`", r.name));
                    } else if let Some(why) = unstable(candidate.symbol) {
                        fixed.insert(format!("refers to itself as `{}`, which {why}", r.name));
                    }
                }
                Some(symbol) => {
                    let declared = *spans
                        .entry(symbol)
                        .or_insert_with(|| scoping.symbol_span(symbol));
                    if within(declared, candidate.node) {
                    } else if let Some(&dep) = by_symbol.get(&symbol) {
                        if r.write {
                            fixed.insert(format!("writes captured `{}`", r.name));
                        } else if let Some(why) = unstable(symbol) {
                            fixed.insert(format!("depends on `{}`, which {why}", r.name));
                        } else {
                            deps.insert(dep);
                        }
                    } else {
                        blockers.insert(format!(
                            "{} captured `{}`",
                            if r.write { "writes" } else { "reads" },
                            r.name
                        ));
                    }
                }
            }
        }
        let lo = member_writes.partition_point(|s| s.start < candidate.node.start);
        for root in member_writes[lo..]
            .iter()
            .take_while(|s| s.start < candidate.node.end)
        {
            if within(*root, candidate.node)
                && let Some(&ri) = ref_start.get(&root.start)
                && refs[ri].symbol.is_none()
            {
                fixed.insert(format!("writes through global `{}`", refs[ri].name));
            }
        }
        if any_in(&this_spans, candidate.node) {
            fixed.insert("uses `this`".to_owned());
        }
        if any_in(&meta_spans, candidate.node) {
            for (span, what) in &meta {
                if within(*span, candidate.node) {
                    fixed.insert(format!("uses {what}"));
                }
            }
        }
        if any_in(&reflective, candidate.node) {
            fixed.insert(
                "reaches `.constructor`, `.__proto__` or `.prototype`, which can lead to the Function constructor"
                    .to_owned(),
            );
        }
        candidate.fixed.extend(fixed.iter().cloned());
        candidate.blockers.extend(fixed);
        candidate.blockers.extend(blockers);
        candidate.deps = deps;
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
        chains,
        member_roots,
        member_writes,
        calls,
        assigns,
        null_init,
        fresh,
        literals,
        guards,
        method_calls,
        imports,
        redeclared,
        mutator_roots,
        frozen_shapes,
        escaping_uses,
        var_declared,
        call_inits,
        escaping_calls,
        fn_spans,
        class_spans,
        any_calls,
        chained_targets,
        literal_bindings,
        decl_numeric,
        exported,
        hazards,
        lines,
        scoping_symbol_spans: spans,
    })
}
