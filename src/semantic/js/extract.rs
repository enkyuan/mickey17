//! oxc extraction in two stages. Stage 1 walks the AST once and lowers what the
//! analysis needs into owned values (`X` expressions, `Fun` records, raw JSX
//! calls), resolving identifiers to oxc `SymbolId`s. Stage 2 is pure: it
//! summarises expressions, follows handler calls through local functions and
//! builds [`JsFacts`]. Nothing matches minified names or source text.
use super::*;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_semantic::{ScopeFlags, Scoping, SemanticBuilder, SymbolId};
use oxc_span::{GetSpan, SourceType, Span};
use std::collections::HashMap;

type FnId = usize;

/// An owned, lowered expression.
#[derive(Debug, Clone)]
enum X {
    Lit(Lit),
    Sym(SymbolId, String),
    Global(String),
    Not(Box<X>),
    Eq(Box<X>, Box<X>),
    Ne(Box<X>, Box<X>),
    Cond(Box<X>, Box<X>, Box<X>),
    And(Box<X>, Box<X>),
    Func(Span),
    Array(Vec<X>),
    Object(Vec<(Option<String>, X)>, Vec<X>),
    Template(Vec<String>, Vec<X>),
    Member(Box<X>, String),
    Index(Box<X>, Box<X>),
    /// A `jsx(..)` call, identified by its source span.
    JsxCall(Span),
    /// Any other call, by its callee.
    Call(Box<X>),
    Other(String),
}

struct CallL {
    span: Span,
    /// Inside an `if`, ternary, logical operand, `switch` or loop of its function.
    conditional: bool,
    callee: X,
    args: Vec<X>,
    line: usize,
    chain: Vec<FnId>,
}
struct AssignL {
    path: Option<String>,
    target: Option<SymbolId>,
    value: X,
    line: usize,
    chain: Vec<FnId>,
}
#[derive(Default)]
struct Fun {
    name: Option<String>,
    decl: Option<SymbolId>,
    params: Vec<Option<SymbolId>>,
    prop_names: Vec<String>,
    calls: Vec<CallL>,
    escape: bool,
    /// The value an arrow function returns (expression body or a lone `return`).
    result: Option<X>,
    /// `Some(b)` when the whole body only runs while `param.matches == b`.
    matches_guard: Option<bool>,
}

#[derive(Default)]
struct Raw {
    funs: Vec<Fun>,
    fn_by_span: HashMap<(u32, u32), FnId>,
    inits: HashMap<SymbolId, X>,
    assigns: HashMap<SymbolId, Vec<X>>,
    assign_records: Vec<AssignL>,
    state_decls: Vec<StateRaw>,
    line_starts: Vec<usize>,
}

struct StateRaw {
    value: SymbolId,
    setter: Option<(SymbolId, String)>,
    name: String,
    init: Option<X>,
    init_source: Option<String>,
    line: usize,
    function: FnId,
}

fn strip<'a, 'b>(mut e: &'b Expression<'a>) -> &'b Expression<'a> {
    loop {
        match e {
            Expression::ParenthesizedExpression(p) => e = &p.expression,
            Expression::SequenceExpression(s) if !s.expressions.is_empty() => {
                e = s.expressions.last().unwrap()
            }
            _ => return e,
        }
    }
}

macro_rules! js {
    ($s:expr) => {
        $s.as_str().map(str::to_owned).unwrap_or_default()
    };
}

fn key_name(key: &PropertyKey<'_>) -> Option<String> {
    key.static_name()
        .and_then(|n| n.as_js_str().as_str().map(str::to_owned))
}

fn line_of(starts: &[usize], offset: u32) -> usize {
    starts.partition_point(|s| *s <= offset as usize)
}

// ---- stage 1 --------------------------------------------------------------

struct Lower<'s> {
    scoping: &'s Scoping,
    source: &'s str,
    raw: Raw,
    stack: Vec<FnId>,
    /// Nesting of conditional constructs inside the current function.
    cond_depth: usize,
}

impl<'s> Lower<'s> {
    fn text(&self, span: Span) -> String {
        self.source[span.start as usize..span.end as usize]
            .chars()
            .take(80)
            .collect()
    }
    fn symbol(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        id.reference_id
            .get()
            .and_then(|r| self.scoping.get_reference(r).symbol_id())
    }
    fn lit(e: &Expression<'_>) -> Option<Lit> {
        Some(match strip(e) {
            Expression::BooleanLiteral(b) => Lit::Bool(b.value),
            Expression::NullLiteral(_) => Lit::Null,
            Expression::NumericLiteral(n) => Lit::Num(n.value),
            Expression::StringLiteral(s) => Lit::Str(js!(s.value)),
            Expression::Identifier(i) if i.name == "undefined" => Lit::Undefined,
            Expression::UnaryExpression(u) => match (u.operator, strip(&u.argument)) {
                (UnaryOperator::LogicalNot, Expression::NumericLiteral(n)) => {
                    Lit::Bool(n.value == 0.0)
                }
                (UnaryOperator::Void, Expression::NumericLiteral(_)) => Lit::Undefined,
                _ => return None,
            },
            _ => return None,
        })
    }
    fn lower(&self, e: &Expression<'_>, depth: usize) -> X {
        if let Some(l) = Self::lit(e) {
            return X::Lit(l);
        }
        if depth > 8 {
            return X::Other(self.text(e.span()));
        }
        let d = depth + 1;
        match strip(e) {
            Expression::Identifier(i) => match self.symbol(i) {
                Some(s) => X::Sym(s, i.name.as_str().to_owned()),
                None => X::Global(i.name.as_str().to_owned()),
            },
            Expression::UnaryExpression(u) if u.operator == UnaryOperator::LogicalNot => {
                X::Not(Box::new(self.lower(&u.argument, d)))
            }
            Expression::BinaryExpression(b) => match b.operator {
                BinaryOperator::StrictEquality | BinaryOperator::Equality => X::Eq(
                    Box::new(self.lower(&b.left, d)),
                    Box::new(self.lower(&b.right, d)),
                ),
                BinaryOperator::StrictInequality | BinaryOperator::Inequality => X::Ne(
                    Box::new(self.lower(&b.left, d)),
                    Box::new(self.lower(&b.right, d)),
                ),
                _ => X::Other(self.text(e.span())),
            },
            Expression::ConditionalExpression(c) => X::Cond(
                Box::new(self.lower(&c.test, d)),
                Box::new(self.lower(&c.consequent, d)),
                Box::new(self.lower(&c.alternate, d)),
            ),
            Expression::LogicalExpression(l) if l.operator == LogicalOperator::And => X::And(
                Box::new(self.lower(&l.left, d)),
                Box::new(self.lower(&l.right, d)),
            ),
            Expression::ArrowFunctionExpression(a) => X::Func(a.span),
            Expression::FunctionExpression(f) => X::Func(f.span),
            Expression::ArrayExpression(a) => X::Array(
                a.elements
                    .iter()
                    .map(|el| match el.as_expression() {
                        Some(x) => self.lower(x, d),
                        None => X::Other("spread".into()),
                    })
                    .collect(),
            ),
            Expression::ObjectExpression(o) => {
                let mut props = Vec::new();
                let mut spreads = Vec::new();
                for p in &o.properties {
                    match p {
                        ObjectPropertyKind::ObjectProperty(p) => {
                            props.push((key_name(&p.key), self.lower(&p.value, d)))
                        }
                        ObjectPropertyKind::SpreadProperty(s) => {
                            spreads.push(self.lower(&s.argument, d))
                        }
                    }
                }
                X::Object(props, spreads)
            }
            Expression::CallExpression(c) => {
                match callee_name(&self.lower(&c.callee, d)).as_deref() {
                    Some("jsx" | "jsxs" | "jsxDEV" | "createElement") => X::JsxCall(c.span),
                    _ => X::Call(Box::new(self.lower(&c.callee, d))),
                }
            }
            Expression::TemplateLiteral(t) => X::Template(
                t.quasis
                    .iter()
                    .map(|q| q.value.cooked.as_ref().map(|c| js!(c)).unwrap_or_default())
                    .collect(),
                t.expressions.iter().map(|x| self.lower(x, d)).collect(),
            ),
            Expression::StaticMemberExpression(m) => X::Member(
                Box::new(self.lower(&m.object, d)),
                m.property.name.as_str().to_owned(),
            ),
            Expression::ComputedMemberExpression(m) => X::Index(
                Box::new(self.lower(&m.object, d)),
                Box::new(self.lower(&m.expression, d)),
            ),
            _ => X::Other(self.text(e.span())),
        }
    }

    fn enter(
        &mut self,
        span: Span,
        name: Option<String>,
        decl: Option<SymbolId>,
        params: &FormalParameters<'_>,
    ) -> FnId {
        let id = self.raw.funs.len();
        self.raw.funs.push(Fun {
            name,
            decl,
            params: params
                .items
                .iter()
                .map(|p| match &p.pattern {
                    BindingPattern::BindingIdentifier(b) => b.symbol_id.get(),
                    _ => None,
                })
                .collect(),
            prop_names: match params.items.first().map(|p| &p.pattern) {
                Some(BindingPattern::ObjectPattern(o)) => o
                    .properties
                    .iter()
                    .filter_map(|p| key_name(&p.key))
                    .collect(),
                _ => Vec::new(),
            },
            ..Default::default()
        });
        self.raw.fn_by_span.insert((span.start, span.end), id);
        self.stack.push(id);
        id
    }
    fn line(&self, span: Span) -> usize {
        line_of(&self.raw.line_starts, span.start)
    }
}

impl<'a, 's> Visit<'a> for Lower<'s> {
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let decl = it
            .id
            .as_ref()
            .filter(|_| it.is_declaration())
            .and_then(|i| i.symbol_id.get());
        let name = it.id.as_ref().map(|i| i.name.as_str().to_owned());
        self.enter(it.span, name, decl, &it.params);
        let saved = std::mem::take(&mut self.cond_depth);
        walk::walk_function(self, it, flags);
        self.cond_depth = saved;
        self.stack.pop();
    }
    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let id = self.enter(it.span, None, None, &it.params);
        let result = if let Some(e) = it.get_expression() {
            Some(self.lower(e, 0))
        } else {
            match &it.body {
                ArrowFunctionBody::FunctionBody(body) => match body.statements.as_slice() {
                    [Statement::ReturnStatement(r)] => {
                        r.argument.as_ref().map(|a| self.lower(a, 0))
                    }
                    [Statement::ExpressionStatement(s)] => Some(self.lower(&s.expression, 0)),
                    _ => None,
                },
                #[allow(unreachable_patterns)]
                _ => None,
            }
        };
        self.raw.funs[id].result = result;
        self.raw.funs[id].matches_guard = Self::guard_of(it);
        let saved = std::mem::take(&mut self.cond_depth);
        walk::walk_arrow_function_expression(self, it);
        self.cond_depth = saved;
        self.stack.pop();
    }
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let Some(init) = &it.init {
            match &it.id {
                BindingPattern::BindingIdentifier(b) => {
                    if let Some(sym) = b.symbol_id.get() {
                        let x = self.lower(init, 0);
                        self.raw.inits.insert(sym, x);
                    }
                }
                BindingPattern::ObjectPattern(o) => {
                    // `const { pathname: t } = useLocation()`: `t` is the member `pathname` of the call.
                    let whole = self.lower(init, 0);
                    for p in &o.properties {
                        if let (Some(key), BindingPattern::BindingIdentifier(b)) =
                            (key_name(&p.key), &p.value)
                            && let Some(sym) = b.symbol_id.get()
                        {
                            self.raw
                                .inits
                                .insert(sym, X::Member(Box::new(whole.clone()), key));
                        }
                    }
                }
                BindingPattern::ArrayPattern(arr) => {
                    if let Expression::CallExpression(call) = strip(init)
                        && callee_name(&self.lower(&call.callee, 0)).as_deref() == Some("useState")
                    {
                        self.record_state(arr, call);
                    }
                }
                _ => {}
            }
        }
        walk::walk_variable_declarator(self, it);
    }
    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        let value = self.lower(&it.right, 0);
        let (target, path) = match &it.left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => (self.symbol(id), None),
            AssignmentTarget::StaticMemberExpression(m) => (
                None,
                path_of(&self.lower(&m.object, 0)).map(|p| format!("{p}.{}", m.property.name)),
            ),
            _ => (None, None),
        };
        if let Some(sym) = target {
            self.raw.assigns.entry(sym).or_default().push(value.clone());
        }
        self.raw.assign_records.push(AssignL {
            path,
            target,
            value,
            line: self.line(it.span),
            chain: self.stack.clone(),
        });
        walk::walk_assignment_expression(self, it);
    }
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let call = CallL {
            span: it.span,
            conditional: self.cond_depth > 0,
            callee: self.lower(&it.callee, 0),
            args: it
                .arguments
                .iter()
                .map(|a| match a.as_expression() {
                    Some(e) => self.lower(e, 0),
                    None => X::Other("spread".into()),
                })
                .collect(),
            line: self.line(it.span),
            chain: self.stack.clone(),
        };
        let owner = self.stack.last().copied().unwrap_or(0);
        self.raw.funs[owner].calls.push(call);
        walk::walk_call_expression(self, it);
    }
    fn visit_statements(&mut self, it: &oxc_allocator::Vec<'a, Statement<'a>>) {
        // Statements after an `if (..) return` run only when the guard did not fire.
        let mut guards = 0;
        for stmt in it {
            self.visit_statement(stmt);
            if returns_early(stmt) {
                self.cond_depth += 1;
                guards += 1;
            }
        }
        self.cond_depth -= guards;
    }
    fn visit_if_statement(&mut self, it: &IfStatement<'a>) {
        self.cond_depth += 1;
        walk::walk_if_statement(self, it);
        self.cond_depth -= 1;
    }
    fn visit_conditional_expression(&mut self, it: &ConditionalExpression<'a>) {
        self.cond_depth += 1;
        walk::walk_conditional_expression(self, it);
        self.cond_depth -= 1;
    }
    fn visit_logical_expression(&mut self, it: &LogicalExpression<'a>) {
        self.cond_depth += 1;
        walk::walk_logical_expression(self, it);
        self.cond_depth -= 1;
    }
    fn visit_switch_statement(&mut self, it: &SwitchStatement<'a>) {
        self.cond_depth += 1;
        walk::walk_switch_statement(self, it);
        self.cond_depth -= 1;
    }
    fn visit_for_statement(&mut self, it: &ForStatement<'a>) {
        self.cond_depth += 1;
        walk::walk_for_statement(self, it);
        self.cond_depth -= 1;
    }
    fn visit_while_statement(&mut self, it: &WhileStatement<'a>) {
        self.cond_depth += 1;
        walk::walk_while_statement(self, it);
        self.cond_depth -= 1;
    }
    fn visit_binary_expression(&mut self, it: &BinaryExpression<'a>) {
        if matches!(
            it.operator,
            BinaryOperator::StrictEquality | BinaryOperator::Equality
        ) {
            let is_escape = |e: &Expression<'a>| matches!(strip(e), Expression::StringLiteral(s) if s.value.as_str() == Some("Escape"));
            let is_key = |e: &Expression<'a>| matches!(strip(e), Expression::StaticMemberExpression(m) if m.property.name == "key");
            if ((is_escape(&it.right) && is_key(&it.left))
                || (is_escape(&it.left) && is_key(&it.right)))
                && let Some(&f) = self.stack.last()
            {
                self.raw.funs[f].escape = true;
            }
        }
        walk::walk_binary_expression(self, it);
    }
}

impl<'s> Lower<'s> {
    /// `p.matches || ..` / `!p.matches && ..` run when the query does not match;
    /// `p.matches && ..` / `if (p.matches)` run when it does.
    fn guard_of(a: &ArrowFunctionExpression<'_>) -> Option<bool> {
        let param = match a.params.items.first().map(|p| &p.pattern) {
            Some(BindingPattern::BindingIdentifier(b)) => b.name.as_str().to_owned(),
            _ => return None,
        };
        let is_matches = |e: &Expression<'_>| match strip(e) {
            Expression::StaticMemberExpression(m) => {
                m.property.name == "matches"
                    && matches!(strip(&m.object), Expression::Identifier(i) if i.name == param.as_str())
            }
            _ => false,
        };
        let from_expr = |e: &Expression<'_>| -> Option<bool> {
            match strip(e) {
                Expression::LogicalExpression(l) => match l.operator {
                    LogicalOperator::Or if is_matches(&l.left) => Some(false),
                    LogicalOperator::And if is_matches(&l.left) => Some(true),
                    LogicalOperator::And => match strip(&l.left) {
                        Expression::UnaryExpression(u)
                            if u.operator == UnaryOperator::LogicalNot
                                && is_matches(&u.argument) =>
                        {
                            Some(false)
                        }
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            }
        };
        let from_if = |t: &Expression<'_>| -> Option<bool> {
            if is_matches(t) {
                return Some(true);
            }
            match strip(t) {
                Expression::UnaryExpression(u)
                    if u.operator == UnaryOperator::LogicalNot && is_matches(&u.argument) =>
                {
                    Some(false)
                }
                _ => None,
            }
        };
        if let Some(e) = a.get_expression() {
            return from_expr(e);
        }
        let ArrowFunctionBody::FunctionBody(body) = &a.body else {
            return None;
        };
        match body.statements.as_slice() {
            [Statement::ExpressionStatement(s)] => from_expr(&s.expression),
            [Statement::IfStatement(i)] if i.alternate.is_none() => from_if(&i.test),
            _ => None,
        }
    }

    fn record_state(&mut self, arr: &ArrayPattern<'_>, call: &CallExpression<'_>) {
        let ident = |p: Option<&Option<BindingPattern<'_>>>| match p {
            Some(Some(BindingPattern::BindingIdentifier(b))) => {
                Some((b.symbol_id.get()?, b.name.as_str().to_owned()))
            }
            _ => None,
        };
        let Some((value, name)) = ident(arr.elements.first()) else {
            return;
        };
        let (init, init_source) = match call.arguments.first().and_then(|a| a.as_expression()) {
            Some(e) => match Self::lit(e) {
                Some(l) => (Some(X::Lit(l)), None),
                None => (None, Some(self.text(e.span()))),
            },
            None => (Some(X::Lit(Lit::Undefined)), None),
        };
        let line = self.line(call.span);
        self.raw.state_decls.push(StateRaw {
            value,
            setter: ident(arr.elements.get(1)),
            name,
            init,
            init_source,
            line,
            function: self.stack.last().copied().unwrap_or(0),
        });
    }
}

/// Every string `x` can produce is empty or begins (`start`) / ends (`!start`) on whitespace.
fn boundary(x: &X, start: bool) -> bool {
    match x {
        X::Lit(Lit::Str(s)) => {
            s.is_empty()
                || if start {
                    s.starts_with(char::is_whitespace)
                } else {
                    s.ends_with(char::is_whitespace)
                }
        }
        X::Lit(Lit::Null | Lit::Undefined | Lit::Bool(false)) => true,
        X::Cond(_, a, b) => boundary(a, start) && boundary(b, start),
        _ => false,
    }
}

/// `if (..) return ..` / `if (..) { ..; return }`: the rest of the block is conditional.
fn returns_early(stmt: &Statement<'_>) -> bool {
    let Statement::IfStatement(i) = stmt else {
        return false;
    };
    match &i.consequent {
        Statement::ReturnStatement(_) => true,
        Statement::BlockStatement(b) => b
            .body
            .iter()
            .any(|s| matches!(s, Statement::ReturnStatement(_))),
        _ => false,
    }
}

fn path_of(x: &X) -> Option<String> {
    match x {
        X::Global(n) | X::Sym(_, n) => Some(n.clone()),
        X::Member(o, p) => Some(format!("{}.{p}", path_of(o)?)),
        _ => None,
    }
}

/// A readable label for an effect dependency: a destructured member's key
/// (`pathname`), else its path, else `expression`.
fn dep_label(an: &Analysis, x: &X) -> String {
    match x {
        X::Sym(s, _) => match an.definition(*s) {
            // Only a member destructured from a call (`const { pathname } = useLocation()`) is a
            // value with a meaningful name; an alias or a prop member is not.
            Some(X::Member(inner, prop)) if matches!(**inner, X::Call(_)) => prop.clone(),
            Some(_) => "(unresolved)".into(),
            // A parameter or a binding with no definition: its minified name means nothing.
            None => "(unresolved)".into(),
        },
        other => path_of(other).unwrap_or_else(|| "(unresolved)".into()),
    }
}

fn callee_name(x: &X) -> Option<String> {
    match x {
        X::Global(n) | X::Sym(_, n) => Some(n.clone()),
        X::Member(_, p) => Some(p.clone()),
        _ => None,
    }
}

// ---- stage 2 --------------------------------------------------------------

type Params = HashMap<SymbolId, Ex>;

struct Analysis {
    raw: Raw,
    state_by_value: HashMap<SymbolId, StateId>,
    state_by_setter: HashMap<SymbolId, StateId>,
    fn_decl: HashMap<SymbolId, FnId>,
    loop_params: Params,
    loops: Vec<LoopInfo>,
    loop_fn: HashMap<FnId, usize>,
}

impl Analysis {
    /// A definition of `sym` that is not the React Compiler's memo-cache read.
    fn definition(&self, sym: SymbolId) -> Option<&X> {
        let mut candidates: Vec<&X> = Vec::new();
        candidates.extend(self.raw.inits.get(&sym));
        if let Some(a) = self.raw.assigns.get(&sym) {
            candidates.extend(a.iter());
        }
        candidates.retain(|x| !matches!(x, X::Index(_, k) if matches!(**k, X::Lit(Lit::Num(_)))));
        (candidates.len() == 1).then(|| candidates[0])
    }
    fn resolve_fn(&self, x: &X, depth: usize) -> Option<FnId> {
        match x {
            X::Func(span) => self.raw.fn_by_span.get(&(span.start, span.end)).copied(),
            X::Sym(s, _) if depth < 6 => self
                .fn_decl
                .get(s)
                .copied()
                .or_else(|| self.resolve_fn(self.definition(*s)?, depth + 1)),
            _ => None,
        }
    }
    fn summarize(&self, x: &X, params: &Params, depth: usize) -> Ex {
        let other = |x: &X| Ex::Other {
            source: format!("{x:?}").chars().take(60).collect(),
        };
        let d = depth + 1;
        match x {
            X::Lit(l) => Ex::Lit { value: l.clone() },
            X::Sym(s, _) => {
                if let Some(ex) = params.get(s).or_else(|| self.loop_params.get(s)) {
                    return ex.clone();
                }
                if let Some(st) = self.state_by_value.get(s) {
                    return Ex::State { state: *st };
                }
                if depth < 6
                    && let Some(def) = self.definition(*s)
                {
                    return self.summarize(def, params, d);
                }
                other(x)
            }
            X::Not(a) => Ex::Not {
                arg: Box::new(self.summarize(a, params, d)),
            },
            X::Eq(a, b) => Ex::Eq {
                left: Box::new(self.summarize(a, params, d)),
                right: Box::new(self.summarize(b, params, d)),
            },
            X::Ne(a, b) => Ex::Not {
                arg: Box::new(Ex::Eq {
                    left: Box::new(self.summarize(a, params, d)),
                    right: Box::new(self.summarize(b, params, d)),
                }),
            },
            X::Cond(t, a, b) => Ex::Cond {
                test: Box::new(self.summarize(t, params, d)),
                then: Box::new(self.summarize(a, params, d)),
                otherwise: Box::new(self.summarize(b, params, d)),
            },
            X::And(a, b) => Ex::And {
                left: Box::new(self.summarize(a, params, d)),
                right: Box::new(self.summarize(b, params, d)),
            },
            X::Index(obj, _) => Ex::Other {
                source: format!("{}[..]", path_of(obj).unwrap_or_default()),
            },
            X::Call(callee) => Ex::Other {
                source: format!("call of {}", path_of(callee).unwrap_or_default()),
            },
            X::Other(text) => Ex::Other {
                source: text.clone(),
            },
            other_x => other(other_x),
        }
    }

    fn effects_of(&self, f: FnId, args: &[Ex], depth: usize) -> Vec<StateEffect> {
        self.effects_flagged(f, args, depth)
            .into_iter()
            .map(|(e, _)| e)
            .collect()
    }

    /// State effects of calling `f`; the flag is `true` for an effect that only happens under a
    /// condition (`if (x) set(..)`, `x && set(..)`), which is not the same as always.
    fn effects_flagged(&self, f: FnId, args: &[Ex], depth: usize) -> Vec<(StateEffect, bool)> {
        let fun = &self.raw.funs[f];
        let mut params = Params::new();
        for (i, p) in fun.params.iter().enumerate() {
            if let (Some(sym), Some(arg)) = (p, args.get(i)) {
                params.insert(*sym, arg.clone());
            }
        }
        let mut out: Vec<(StateEffect, bool)> = Vec::new();
        for call in &fun.calls {
            // Only calls lexically owned by this function (not its nested functions).
            if call.chain.last() != Some(&f) {
                continue;
            }
            let X::Sym(sym, _) = &call.callee else {
                continue;
            };
            if let Some(&state) = self.state_by_setter.get(sym) {
                out.extend(
                    self.setter_effect(state, call.args.first(), &params, depth)
                        .into_iter()
                        .map(|(e, c)| (e, c || call.conditional)),
                );
            } else if depth < 3 {
                let target = self
                    .fn_decl
                    .get(sym)
                    .copied()
                    .or_else(|| self.definition(*sym).and_then(|d| self.resolve_fn(d, 0)));
                if let Some(t) = target {
                    let args: Vec<Ex> = call
                        .args
                        .iter()
                        .map(|a| self.summarize(a, &params, 0))
                        .collect();
                    out.extend(
                        self.effects_flagged(t, &args, depth + 1)
                            .into_iter()
                            .map(|(e, c)| (e, c || call.conditional)),
                    );
                }
            }
        }
        out
    }

    fn setter_effect(
        &self,
        state: StateId,
        arg: Option<&X>,
        params: &Params,
        depth: usize,
    ) -> Vec<(StateEffect, bool)> {
        let Some(arg) = arg else {
            return vec![(
                StateEffect::Other {
                    state,
                    source: "no argument".into(),
                },
                false,
            )];
        };
        if let Some(f) = self.resolve_fn(arg, 0) {
            // Updater: its own nested state moves count too (`u && c(null)`).
            let mut out = if depth < 3 {
                self.effects_flagged(f, &[], depth + 1)
            } else {
                Vec::new()
            };
            let fun = &self.raw.funs[f];
            let param = fun.params.first().copied().flatten();
            let is_param = |x: &X| matches!((x, param), (X::Sym(s, _), Some(p)) if *s == p);
            let main = match &fun.result {
                Some(X::Not(a)) if is_param(a) => Some(StateEffect::Toggle { state }),
                Some(X::Cond(t, a, b)) => match &**t {
                    X::Eq(l, r)
                        if is_param(l)
                            && matches!(
                                **a,
                                X::Lit(Lit::Null | Lit::Undefined | Lit::Bool(false))
                            ) =>
                    {
                        // `v === k ? null : k`: k a parameter/variable, or the same literal twice.
                        let same = match (&**r, &**b) {
                            (X::Sym(x, _), X::Sym(y, _)) => x == y,
                            (X::Lit(x), X::Lit(y)) => x == y,
                            _ => false,
                        };
                        match (same, self.summarize(r, params, 0)) {
                            (true, Ex::Key { key }) => Some(StateEffect::ToggleKey { state, key }),
                            (true, Ex::Lit { value }) => Some(StateEffect::ToggleKey {
                                state,
                                key: KeyRef::Literal { value },
                            }),
                            _ => None,
                        }
                    }
                    _ => None,
                },
                _ => None,
            };
            out.push((
                main.unwrap_or(StateEffect::Other {
                    state,
                    source: "updater function".into(),
                }),
                false,
            ));
            return out;
        }
        vec![(
            match self.summarize(arg, params, 0) {
                Ex::Key { key } => StateEffect::SetKey { state, key },
                Ex::Not { arg } if matches!(*arg, Ex::State { state: s } if s == state) => {
                    StateEffect::Toggle { state }
                }
                value => StateEffect::Set { state, value },
            },
            false,
        )]
    }

    fn class_info(&self, x: &X, site: &mut Site) {
        match x {
            X::Lit(Lit::Str(s)) => site
                .class_tokens
                .extend(s.split_whitespace().map(str::to_owned)),
            X::Template(quasis, exprs) => {
                let params = Params::new();
                for (i, text) in quasis.iter().enumerate() {
                    let mut tokens: Vec<&str> = text.split_whitespace().collect();
                    // A token glued to a neighbouring `${..}` is complete only when every
                    // value of that expression is empty or starts/ends on whitespace.
                    if i < exprs.len()
                        && !text.is_empty()
                        && !text.ends_with(char::is_whitespace)
                        && !boundary(&exprs[i], true)
                    {
                        tokens.pop();
                    }
                    if i > 0
                        && !text.is_empty()
                        && !text.starts_with(char::is_whitespace)
                        && !boundary(&exprs[i - 1], false)
                        && !tokens.is_empty()
                    {
                        tokens.remove(0);
                    }
                    site.class_tokens
                        .extend(tokens.into_iter().map(str::to_owned));
                    if let Some(X::Cond(test, a, b)) = exprs.get(i)
                        && i < exprs.len()
                    {
                        // A branch glued to preceding text (`menu--${o ? "open" : "closed"}`) is
                        // a suffix of that class, not a class of its own.
                        let glued = !text.is_empty() && !text.ends_with(char::is_whitespace);
                        let t = self.summarize(test, &params, 0);
                        let toks = |x: &X| match x {
                            X::Lit(Lit::Str(s)) => {
                                let mut v: Vec<String> =
                                    s.split_whitespace().map(str::to_owned).collect();
                                if glued && !s.starts_with(char::is_whitespace) && !v.is_empty() {
                                    v.remove(0);
                                }
                                v
                            }
                            _ => Vec::new(),
                        };
                        for tok in toks(a) {
                            site.class_conditional.push((tok, t.clone()));
                        }
                        for tok in toks(b) {
                            site.class_conditional.push((
                                tok,
                                Ex::Not {
                                    arg: Box::new(t.clone()),
                                },
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Spans of the `jsx(..)` calls an expression (a `children` prop) refers to,
    /// following variables the React Compiler hoisted them into.
    fn child_spans(&self, x: &X, depth: usize) -> Vec<Span> {
        if depth > 6 {
            return Vec::new();
        }
        match x {
            X::JsxCall(s) => vec![*s],
            X::Array(items) => items
                .iter()
                .flat_map(|i| self.child_spans(i, depth + 1))
                .collect(),
            X::Cond(_, a, b) => {
                let mut v = self.child_spans(a, depth + 1);
                v.extend(self.child_spans(b, depth + 1));
                v
            }
            X::And(_, b) => self.child_spans(b, depth + 1),
            X::Sym(sym, _) => self
                .definition(*sym)
                .map(|d| self.child_spans(d, depth + 1))
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn make_site(&self, id: usize, call: &CallL, child_spans: &mut Vec<Span>) -> Option<Site> {
        let tag_x = call.args.first()?;
        let (tag, intrinsic) = match tag_x {
            X::Lit(Lit::Str(s)) => (s.clone(), true),
            X::Sym(_, n) | X::Global(n) => (n.clone(), false),
            X::Member(..) => (path_of(tag_x)?, false),
            _ => return None,
        };
        let loops = call
            .chain
            .iter()
            .filter_map(|f| self.loop_fn.get(f).copied())
            .collect();
        let mut site = Site {
            id,
            line: call.line,
            tag,
            intrinsic,
            class_tokens: Vec::new(),
            class_conditional: Vec::new(),
            static_props: Default::default(),
            bindings: Vec::new(),
            conditional_props: Vec::new(),
            handlers: Vec::new(),
            function: call.chain.last().copied().unwrap_or(0),
            loops,
            children: Vec::new(),
            parent: None,
        };
        let params = Params::new();
        if let Some(X::Object(props, spreads)) = call.args.get(1) {
            for (key, value) in props {
                let Some(key) = key else { continue };
                match key.as_str() {
                    "ref" | "key" => {}
                    "children" => child_spans.extend(self.child_spans(value, 0)),
                    "className" => self.class_info(value, &mut site),
                    k if k.len() > 2
                        && k.starts_with("on")
                        && k.as_bytes()[2].is_ascii_uppercase() =>
                    {
                        if let Some(f) = self.resolve_fn(value, 0) {
                            site.handlers.push(Handler {
                                event: k.to_owned(),
                                effects: self.effects_of(f, &[], 0),
                            });
                        }
                    }
                    _ => match self.summarize(value, &params, 0) {
                        Ex::Lit { value: Lit::Str(s) } => {
                            site.static_props.insert(key.clone(), s);
                        }
                        Ex::Lit {
                            value: Lit::Bool(b),
                        } => {
                            site.static_props.insert(key.clone(), b.to_string());
                        }
                        Ex::Lit { value: Lit::Num(n) } => {
                            site.static_props.insert(key.clone(), n.to_string());
                        }
                        ex => site.bindings.push((key.clone(), ex)),
                    },
                }
            }
            for s in spreads {
                if let X::And(cond, obj) = s
                    && let X::Object(inner, _) = &**obj
                {
                    let cond = self.summarize(cond, &params, 0);
                    let props = inner
                        .iter()
                        .filter_map(|(k, v)| Some((k.clone()?, self.summarize(v, &params, 0))))
                        .collect();
                    site.conditional_props.push((cond, props));
                }
            }
        }
        Some(site)
    }
}

/// Parse `source` and recover the facts. A source that does not parse yields an
/// empty fact set with `parse_errors` set; nothing is guessed.
pub fn extract(file: &str, source: &str) -> JsFacts {
    let arena = Allocator::default();
    let parsed = Parser::new(&arena, source, SourceType::mjs()).parse();
    if !parsed.diagnostics.is_empty() {
        return JsFacts {
            file: file.to_owned(),
            parse_errors: parsed.diagnostics.len().max(1),
            ..Default::default()
        };
    }
    let built = SemanticBuilder::new_compiler().build(&parsed.program);
    let scoping = built.semantic.scoping();
    let mut line_starts = vec![0usize];
    line_starts.extend(
        source
            .bytes()
            .enumerate()
            .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
    );
    let mut lower = Lower {
        scoping,
        source,
        raw: Raw {
            line_starts,
            ..Default::default()
        },
        stack: Vec::new(),
        cond_depth: 0,
    };
    // Function 0 is the module itself so top-level statements have an owner.
    lower.raw.funs.push(Fun::default());
    lower.stack.push(0);
    lower.visit_program(&parsed.program);
    let raw = lower.raw;

    let mut an = Analysis {
        state_by_value: raw
            .state_decls
            .iter()
            .enumerate()
            .map(|(i, s)| (s.value, i))
            .collect(),
        state_by_setter: raw
            .state_decls
            .iter()
            .enumerate()
            .filter_map(|(i, s)| Some((s.setter.as_ref()?.0, i)))
            .collect(),
        fn_decl: raw
            .funs
            .iter()
            .enumerate()
            .filter_map(|(i, f)| Some((f.decl?, i)))
            .collect(),
        loop_params: Params::new(),
        loops: Vec::new(),
        loop_fn: HashMap::new(),
        raw,
    };

    // Loops: `<array>.map(cb)`, with static item lists when the array is a literal.
    let mut loop_specs: Vec<(usize, FnId, Option<Vec<String>>, String)> = Vec::new();
    for fun in &an.raw.funs {
        for call in &fun.calls {
            let X::Member(obj, prop) = &call.callee else {
                continue;
            };
            if prop != "map" {
                continue;
            }
            let Some(cb) = call.args.first().and_then(|a| an.resolve_fn(a, 0)) else {
                continue;
            };
            let items = match &**obj {
                X::Sym(s, _) => an.definition(*s).and_then(|d| match d {
                    X::Array(items) => items
                        .iter()
                        .map(|i| match i {
                            X::Lit(Lit::Str(s)) => Some(s.clone()),
                            _ => None,
                        })
                        .collect::<Option<Vec<_>>>(),
                    _ => None,
                }),
                _ => None,
            };
            let source_text = path_of(obj).unwrap_or_else(|| "expression".into());
            loop_specs.push((call.line, cb, items, source_text));
        }
    }
    for (line, cb, items, source_text) in loop_specs {
        if an.loop_fn.contains_key(&cb) {
            continue;
        }
        let id = an.loops.len();
        an.loops.push(LoopInfo {
            id,
            line,
            items: items.clone(),
            source: source_text,
        });
        an.loop_fn.insert(cb, id);
        let params = an.raw.funs[cb].params.clone();
        if let Some(Some(sym)) = params.first() {
            an.loop_params.insert(
                *sym,
                Ex::Key {
                    key: KeyRef::LoopItem { loop_id: id, items },
                },
            );
        }
        if let Some(Some(sym)) = params.get(1) {
            an.loop_params.insert(
                *sym,
                Ex::Key {
                    key: KeyRef::LoopIndex { loop_id: id },
                },
            );
        }
    }

    let mut out = JsFacts {
        file: file.to_owned(),
        loops: an.loops.clone(),
        ..Default::default()
    };
    for (i, s) in an.raw.state_decls.iter().enumerate() {
        out.states.push(StateVar {
            id: i,
            name: s.name.clone(),
            setter: s.setter.as_ref().map(|x| x.1.clone()).unwrap_or_default(),
            initial: match &s.init {
                Some(X::Lit(l)) => Some(l.clone()),
                _ => None,
            },
            initial_source: s.init_source.clone(),
            line: s.line,
            function: s.function,
        });
    }
    out.functions = an
        .raw
        .funs
        .iter()
        .enumerate()
        .map(|(id, f)| FnInfo {
            id,
            name: f.name.clone(),
            prop_names: f.prop_names.clone(),
        })
        .collect();

    // JSX sites and effect-like facts, from every recorded call.
    let calls: Vec<&CallL> = an.raw.funs.iter().flat_map(|f| f.calls.iter()).collect();
    let mut site_by_span: HashMap<(u32, u32), usize> = HashMap::new();
    let mut spans_of_children: Vec<Vec<Span>> = Vec::new();
    for call in calls {
        match callee_name(&call.callee).as_deref() {
            Some("jsx" | "jsxs" | "jsxDEV" | "createElement") => {
                let mut kids = Vec::new();
                if let Some(site) = an.make_site(out.sites.len(), call, &mut kids) {
                    site_by_span.insert((call.span.start, call.span.end), site.id);
                    spans_of_children.push(kids);
                    out.sites.push(site);
                }
            }
            Some("addEventListener") => {
                let (Some(X::Lit(Lit::Str(kind))), Some(listener)) =
                    (call.args.first(), call.args.get(1))
                else {
                    continue;
                };
                let Some(f) = an.resolve_fn(listener, 0) else {
                    continue;
                };
                if kind == "keydown" && an.raw.funs[f].escape {
                    out.effects.push(EffectRecord {
                        line: call.line,
                        chain: call.chain.clone(),
                        fact: EffectFact::EscapeKey {
                            effects: an.effects_of(f, &[], 0),
                        },
                    });
                }
            }
            Some("matchMedia") => {
                let Some(X::Lit(Lit::Str(query))) = call.args.first() else {
                    continue;
                };
                let Some(&owner) = call.chain.last() else {
                    continue;
                };
                // The `change` listener must be registered in the same function.
                let mut effects = Vec::new();
                let mut guard: Option<bool> = None;
                for c in &an.raw.funs[owner].calls {
                    if callee_name(&c.callee).as_deref() == Some("addEventListener")
                        && matches!(c.args.first(), Some(X::Lit(Lit::Str(k))) if k == "change")
                        && let Some(f) = c.args.get(1).and_then(|l| an.resolve_fn(l, 0))
                    {
                        effects.extend(an.effects_of(f, &[], 0));
                        guard = guard.or(an.raw.funs[f].matches_guard);
                    }
                }
                if !effects.is_empty() {
                    out.effects.push(EffectRecord {
                        line: call.line,
                        chain: call.chain.clone(),
                        fact: EffectFact::MediaQuery {
                            query: query.clone(),
                            when_matches: guard,
                            effects,
                        },
                    });
                }
            }
            Some("useEffect" | "useLayoutEffect") => {
                let Some(body) = call.args.first() else {
                    continue;
                };
                let deps: Vec<X> = match call.args.get(1) {
                    Some(X::Array(d)) => d.clone(),
                    Some(X::Sym(s, _)) => match an.definition(*s) {
                        Some(X::Array(d)) => d.clone(),
                        _ => continue,
                    },
                    _ => continue,
                };
                let Some(f) = an.resolve_fn(body, 0) else {
                    continue;
                };
                let params = Params::new();
                let names: Vec<String> = deps
                    .iter()
                    .filter(|d| !matches!(an.summarize(d, &params, 0), Ex::State { .. }))
                    .map(|d| dep_label(&an, d))
                    .collect();
                let flagged = an.effects_flagged(f, &[], 0);
                let unconditional = flagged.iter().all(|(_, c)| !c);
                let effects: Vec<StateEffect> = flagged.into_iter().map(|(e, _)| e).collect();
                if !names.is_empty() && !effects.is_empty() {
                    out.effects.push(EffectRecord {
                        line: call.line,
                        chain: call.chain.clone(),
                        fact: EffectFact::DepsChange {
                            deps: names,
                            unconditional,
                            effects,
                        },
                    });
                }
            }
            _ => {}
        }
    }
    for (id, kids) in spans_of_children.iter().enumerate() {
        let ids: Vec<usize> = kids
            .iter()
            .filter_map(|s| site_by_span.get(&(s.start, s.end)).copied())
            .collect();
        for k in &ids {
            out.sites[*k].parent = Some(id);
        }
        out.sites[id].children = ids;
    }
    for a in &an.raw.assign_records {
        if a.path.as_deref() == Some("document.body.style.overflow")
            && let Ex::Cond { test, then, .. } = an.summarize(&a.value, &Params::new(), 0)
            && let Ex::State { state } = *test
            // The lock applies while the state is truthy: the truthy branch must hide.
            && matches!(*then, Ex::Lit { value: Lit::Str(ref v) } if v == "hidden")
        {
            out.effects.push(EffectRecord {
                line: a.line,
                chain: a.chain.clone(),
                fact: EffectFact::ScrollLock { state },
            });
        }
        let _ = a.target;
    }
    out
}
