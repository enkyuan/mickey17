//! Binding-level recovery for split Vite React components. Unrelated application
//! bootstrap code is excluded; selected declarations and their mutations retain order.
use super::*;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::{GetSpan, SourceType, Span};

#[derive(Clone, Debug)]
struct Fragment {
    span: Span,
    code: String,
    deps: BTreeSet<String>,
    writes: BTreeSet<String>,
    bridge: Option<Bridge>,
}
#[derive(Clone, Debug)]
enum Bridge {
    React,
    Jsx,
    Compiler,
    Dom,
    Router(&'static str),
    Outlet,
    Data(String),
}
#[derive(Clone, Debug, PartialEq)]
enum Literal {
    Json(Value),
    Undefined,
    Array(Vec<Literal>),
    Object(Vec<(String, Literal)>),
}
impl Literal {
    fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(p) => p.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn keys(&self) -> BTreeSet<String> {
        match self {
            Self::Object(p) => p.iter().map(|(k, _)| k.clone()).collect(),
            _ => BTreeSet::new(),
        }
    }
    fn string(&self) -> Option<&str> {
        match self {
            Self::Json(Value::String(s)) => Some(s),
            _ => None,
        }
    }
    fn source(&self) -> Result<String> {
        Ok(match self {
            Self::Json(v) => serde_json::to_string(v)?,
            Self::Undefined => "void 0".into(),
            Self::Array(a) => format!(
                "[{}]",
                a.iter()
                    .map(Self::source)
                    .collect::<Result<Vec<_>>>()?
                    .join(",")
            ),
            Self::Object(o) => format!(
                "{{{}}}",
                o.iter()
                    .map(|(k, v)| Ok(format!("[{}]:{}", serde_json::to_string(k)?, v.source()?)))
                    .collect::<Result<Vec<_>>>()?
                    .join(",")
            ),
        })
    }
}
fn reference(object: &Expression<'_>, property: &Expression<'_>) -> Option<String> {
    let Expression::Identifier(id) = object else {
        return None;
    };
    let property = match property {
        Expression::NumericLiteral(n) => n.value.to_string(),
        Expression::StringLiteral(s) => s.value.as_str()?.into(),
        _ => return None,
    };
    Some(format!("{}:{property}", id.name))
}
#[derive(Default)]
struct Catalog<'a> {
    references: BTreeMap<String, &'a Expression<'a>>,
    objects: Vec<&'a ObjectExpression<'a>>,
}
impl<'a> Visit<'a> for Catalog<'a> {
    fn visit_assignment_expression(&mut self, e: &AssignmentExpression<'a>) {
        if let AssignmentTarget::ComputedMemberExpression(m) = &e.left
            && let Some(key) = reference(&m.object, &m.expression)
        {
            self.references.insert(key, self.alloc(&e.right));
        }
        walk::walk_assignment_expression(self, e);
    }
    fn visit_object_expression(&mut self, e: &ObjectExpression<'a>) {
        self.objects.push(self.alloc(e));
        walk::walk_object_expression(self, e);
    }
}
fn literal(
    expression: &Expression<'_>,
    catalog: &Catalog<'_>,
    depth: usize,
    budget: &mut usize,
) -> Result<Literal> {
    ensure!(depth < 64, "serialized data depth exceeded");
    *budget = budget
        .checked_sub(64)
        .context("serialized data expansion budget exceeded")?;
    Ok(match expression {
        Expression::StringLiteral(s) => {
            let text = s.value.as_str().context("non-UTF8 data")?;
            *budget = budget
                .checked_sub(text.len())
                .context("serialized data expansion budget exceeded")?;
            Literal::Json(Value::String(text.into()))
        }
        Expression::BooleanLiteral(b) => Literal::Json(Value::Bool(b.value)),
        Expression::NullLiteral(_) => Literal::Json(Value::Null),
        Expression::NumericLiteral(n) => Literal::Json(serde_json::json!(n.value)),
        Expression::AssignmentExpression(a) if a.operator.as_str() == "=" => {
            literal(&a.right, catalog, depth + 1, budget)?
        }
        Expression::ParenthesizedExpression(p) => {
            literal(&p.expression, catalog, depth + 1, budget)?
        }
        Expression::UnaryExpression(u)
            if u.operator.as_str() == "void"
                && matches!(u.argument, Expression::NumericLiteral(_)) =>
        {
            Literal::Undefined
        }
        Expression::UnaryExpression(u) if u.operator.as_str() == "!" => {
            let value = literal(&u.argument, catalog, depth + 1, budget)?;
            let truth = match value {
                Literal::Undefined | Literal::Json(Value::Null) => false,
                Literal::Json(Value::Bool(b)) => b,
                Literal::Json(Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
                Literal::Json(Value::String(s)) => !s.is_empty(),
                _ => true,
            };
            Literal::Json(Value::Bool(!truth))
        }
        Expression::UnaryExpression(u) if u.operator.as_str() == "-" => {
            let Literal::Json(Value::Number(n)) = literal(&u.argument, catalog, depth + 1, budget)?
            else {
                anyhow::bail!("nonliteral numeric data")
            };
            Literal::Json(serde_json::json!(-n.as_f64().context("invalid number")?))
        }
        Expression::ComputedMemberExpression(m) => {
            let key = reference(&m.object, &m.expression).context("dynamic data reference")?;
            literal(
                catalog
                    .references
                    .get(&key)
                    .context("unresolved data reference")?,
                catalog,
                depth + 1,
                budget,
            )?
        }
        Expression::ArrayExpression(a) => {
            let mut values = Vec::new();
            for element in &a.elements {
                values.push(literal(
                    element.as_expression().context("nonliteral data array")?,
                    catalog,
                    depth + 1,
                    budget,
                )?);
            }
            Literal::Array(values)
        }
        Expression::ObjectExpression(o) => {
            let mut values = Vec::new();
            for property in &o.properties {
                let ObjectPropertyKind::ObjectProperty(p) = property else {
                    anyhow::bail!("nonliteral data spread")
                };
                ensure!(
                    p.kind == PropertyKind::Init && !p.method,
                    "executable serialized data"
                );
                let key = p
                    .key
                    .static_name()
                    .and_then(|k| k.into_cow_str())
                    .context("dynamic data key")?;
                *budget = budget
                    .checked_sub(key.len())
                    .context("serialized data expansion budget exceeded")?;
                values.push((
                    key.into_owned(),
                    literal(&p.value, catalog, depth + 1, budget)?,
                ));
            }
            Literal::Object(values)
        }
        _ => anyhow::bail!("unsupported serialized expression"),
    })
}
fn catalog(html: &str) -> Result<Vec<Literal>> {
    let scripts = regex::Regex::new(r"(?s)<script[^>]*data-tsr-stream-part[^>]*>(.*?)</script>")?;
    let mut values = Vec::new();
    let mut budget = 64 * 1024 * 1024;
    for capture in scripts.captures_iter(html) {
        let source = &capture[1];
        let arena = Allocator::default();
        let parsed = Parser::new(&arena, source, SourceType::cjs()).parse();
        ensure!(
            parsed.diagnostics.is_empty(),
            "invalid serialized router state"
        );
        turbopack::format_shape(&parsed.program)?;
        let mut catalog = Catalog::default();
        catalog.visit_program(&parsed.program);
        for object in &catalog.objects {
            let mut properties = Vec::new();
            let mut valid = true;
            for p in &object.properties {
                let ObjectPropertyKind::ObjectProperty(p) = p else {
                    valid = false;
                    break;
                };
                let Some(key) = p.key.static_name().and_then(|k| k.into_cow_str()) else {
                    valid = false;
                    break;
                };
                match literal(&p.value, &catalog, 0, &mut budget) {
                    Ok(value) => properties.push((key.into_owned(), value)),
                    Err(error) => {
                        if error.to_string().contains("expansion budget") {
                            return Err(error);
                        }
                        valid = false;
                        break;
                    }
                }
            }
            if valid {
                values.push(Literal::Object(properties));
            }
        }
    }
    ensure!(values.len() < 10000, "serialized object limit");
    Ok(values)
}
fn data(catalog: &[Literal], keys: &BTreeSet<String>) -> Result<Literal> {
    let mut matches = catalog
        .iter()
        .filter(|v| keys.is_subset(&v.keys()))
        .cloned()
        .collect::<Vec<_>>();
    matches.dedup();
    ensure!(
        matches.len() == 1,
        "ambiguous or missing captured loader data"
    );
    Ok(matches.remove(0))
}
#[derive(Clone, Debug)]
struct Import {
    source: String,
    name: String,
}
#[derive(Debug)]
struct Module {
    fragments: Vec<Fragment>,
    bindings: BTreeMap<String, usize>,
    imports: BTreeMap<String, Import>,
    exports: BTreeMap<String, String>,
    loader_keys: BTreeMap<String, BTreeSet<String>>,
    class_names: BTreeSet<String>,
    mutations: BTreeMap<String, Vec<usize>>,
}
fn name(name: &ModuleExportName<'_>) -> Result<String> {
    match name {
        ModuleExportName::IdentifierName(i) => Ok(i.name.to_string()),
        ModuleExportName::IdentifierReference(i) => Ok(i.name.to_string()),
        ModuleExportName::StringLiteral(s) => {
            Ok(s.value.as_str().context("non-UTF8 export")?.into())
        }
    }
}
struct References<'s> {
    scoping: &'s Scoping,
    names: BTreeSet<String>,
    writes: BTreeSet<String>,
}
impl References<'_> {
    fn identifier(&self, id: &IdentifierReference<'_>) -> Option<String> {
        let symbol = self
            .scoping
            .get_reference(id.reference_id.get()?)
            .symbol_id()?;
        (self.scoping.symbol_scope_id(symbol) == self.scoping.root_scope_id())
            .then(|| self.scoping.symbol_name(symbol).into())
    }
    fn base(&self, e: &Expression<'_>) -> Option<String> {
        match e {
            Expression::Identifier(i) => self.identifier(i),
            Expression::StaticMemberExpression(m) => self.base(&m.object),
            Expression::ComputedMemberExpression(m) => self.base(&m.object),
            _ => None,
        }
    }
}
impl<'a> Visit<'a> for References<'_> {
    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        if let Some(name) = self.identifier(id) {
            self.names.insert(name);
        }
    }
    fn visit_assignment_expression(&mut self, e: &AssignmentExpression<'a>) {
        let root = match &e.left {
            AssignmentTarget::AssignmentTargetIdentifier(i) => self.identifier(i),
            AssignmentTarget::StaticMemberExpression(m) => self.base(&m.object),
            AssignmentTarget::ComputedMemberExpression(m) => self.base(&m.object),
            _ => None,
        };
        if let Some(name) = root {
            self.writes.insert(name);
        }
        walk::walk_assignment_expression(self, e);
    }
    fn visit_call_expression(&mut self, e: &CallExpression<'a>) {
        if let Expression::StaticMemberExpression(m) = &e.callee
            && let Expression::Identifier(i) = &m.object
            && i.name == "Object"
            && matches!(
                m.property.name.as_str(),
                "assign" | "defineProperty" | "defineProperties" | "setPrototypeOf"
            )
            && let Some(arg) = e.arguments.first().and_then(Argument::as_expression)
            && let Some(name) = self.base(arg)
        {
            self.writes.insert(name);
        }
        walk::walk_call_expression(self, e);
    }
}
struct Metadata {
    keys: BTreeMap<String, BTreeSet<String>>,
    classes: BTreeSet<String>,
}
impl<'a> Visit<'a> for Metadata {
    fn visit_variable_declarator(&mut self, d: &VariableDeclarator<'a>) {
        if let BindingPattern::ObjectPattern(pattern) = &d.id
            && let Some(Expression::CallExpression(call)) = &d.init
            && let Expression::StaticMemberExpression(m) = &call.callee
            && m.property.name == "useLoaderData"
            && let Expression::Identifier(id) = &m.object
        {
            let keys = self.keys.entry(id.name.to_string()).or_default();
            for prop in &pattern.properties {
                if let Some(key) = prop.key.static_name()
                    && let Some(key) = key.as_str()
                {
                    keys.insert(key.to_owned());
                }
            }
        }
        walk::walk_variable_declarator(self, d);
    }
    fn visit_object_property(&mut self, p: &ObjectProperty<'a>) {
        if p.key
            .static_name()
            .is_some_and(|x| x == "className" || x == "class")
            && let Expression::StringLiteral(s) = &p.value
            && let Some(s) = s.value.as_str()
        {
            self.classes.extend(s.split_whitespace().map(str::to_owned));
        }
        walk::walk_object_property(self, p);
    }
}
fn bridge(code: &str) -> Option<Bridge> {
    if code.starts_with("function ") && code.contains(".version=\"19.2.7\"") {
        if code.contains(".useState=")
            && code.contains(".useEffect=")
            && code.contains(".createElement=")
        {
            return Some(Bridge::React);
        }
        if code.contains(".createPortal=") && code.contains(".flushSync=") {
            return Some(Bridge::Dom);
        }
    }
    if code.starts_with("function ")
        && code.contains(".jsx=")
        && code.contains(".jsxs=")
        && code.contains("react.transitional.element")
    {
        return Some(Bridge::Jsx);
    }
    if code.starts_with("function ") && code.contains("useMemoCache") && code.contains(".c=") {
        return Some(Bridge::Compiler);
    }
    if code.contains(".stores.location") && code.starts_with("function ") {
        return Some(Bridge::Router("useLocation"));
    }
    if code.contains(".stores.__store") && code.starts_with("function ") {
        return Some(Bridge::Router("useRouterState"));
    }
    if code.contains("_asChild:") && code.contains(".forwardRef(") && code.starts_with("function ")
    {
        return Some(Bridge::Router("createLink"));
    }
    if code.contains(".getMatchStore(")
        && code.contains(".indexOf(")
        && code.contains(".routesById[")
        && code.contains(".Suspense")
    {
        return Some(Bridge::Outlet);
    }
    None
}
impl Module {
    fn parse(source: &str) -> Result<Self> {
        let arena = Allocator::default();
        let parsed = Parser::new(&arena, source, SourceType::mjs()).parse();
        ensure!(parsed.diagnostics.is_empty(), "invalid Vite module");
        turbopack::format_shape(&parsed.program)?;
        let sema = SemanticBuilder::new_compiler().build(&parsed.program);
        ensure!(sema.diagnostics.is_empty(), "invalid Vite bindings");
        let scopes = sema.semantic.scoping();
        let mut module = Self {
            fragments: Vec::new(),
            bindings: BTreeMap::new(),
            imports: BTreeMap::new(),
            exports: BTreeMap::new(),
            loader_keys: BTreeMap::new(),
            class_names: BTreeSet::new(),
            mutations: BTreeMap::new(),
        };
        for statement in &parsed.program.body {
            match statement {
                Statement::ImportDeclaration(i) => {
                    let source = i
                        .source
                        .value
                        .as_str()
                        .context("non-UTF8 import")?
                        .to_owned();
                    for spec in i.specifiers.iter().flatten() {
                        match spec {
                            ImportDeclarationSpecifier::ImportSpecifier(s) => {
                                module.imports.insert(
                                    s.local.name.to_string(),
                                    Import {
                                        source: source.clone(),
                                        name: name(&s.imported)?,
                                    },
                                );
                            }
                            ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                                module.imports.insert(
                                    s.local.name.to_string(),
                                    Import {
                                        source: source.clone(),
                                        name: "default".into(),
                                    },
                                );
                            }
                            _ => anyhow::bail!("namespace imports require an explicit adapter"),
                        }
                    }
                }
                Statement::ExportNamedDeclaration(e) => {
                    for spec in &e.specifiers {
                        module
                            .exports
                            .insert(name(&spec.exported)?, name(&spec.local)?);
                    }
                }
                Statement::VariableDeclaration(v) => {
                    for d in &v.declarations {
                        let mut refs = References {
                            scoping: scopes,
                            names: BTreeSet::new(),
                            writes: BTreeSet::new(),
                        };
                        refs.visit_variable_declarator(d);
                        let code = format!("{} {};", v.kind.as_str(), d.span.source_text(source));
                        module.fragments.push(Fragment {
                            span: d.span,
                            bridge: bridge(&code),
                            code,
                            deps: refs.names,
                            writes: BTreeSet::new(),
                        });
                    }
                }
                Statement::FunctionDeclaration(f) => {
                    let mut refs = References {
                        scoping: scopes,
                        names: BTreeSet::new(),
                        writes: BTreeSet::new(),
                    };
                    refs.visit_statement(statement);
                    let code = f.span.source_text(source).to_owned();
                    module.fragments.push(Fragment {
                        span: f.span,
                        bridge: bridge(&code),
                        code,
                        deps: refs.names,
                        writes: BTreeSet::new(),
                    });
                }
                Statement::ClassDeclaration(c) => {
                    let mut refs = References {
                        scoping: scopes,
                        names: BTreeSet::new(),
                        writes: BTreeSet::new(),
                    };
                    refs.visit_statement(statement);
                    module.fragments.push(Fragment {
                        span: c.span,
                        bridge: None,
                        code: c.span.source_text(source).into(),
                        deps: refs.names,
                        writes: BTreeSet::new(),
                    });
                }
                Statement::ExpressionStatement(e) => {
                    let expressions = match &e.expression {
                        Expression::SequenceExpression(s) => {
                            s.expressions.iter().collect::<Vec<_>>()
                        }
                        e => vec![e],
                    };
                    for e in expressions {
                        let mut refs = References {
                            scoping: scopes,
                            names: BTreeSet::new(),
                            writes: BTreeSet::new(),
                        };
                        refs.visit_expression(e);
                        if !refs.writes.is_empty() {
                            module.fragments.push(Fragment {
                                span: e.span(),
                                bridge: None,
                                code: format!("{};", e.span().source_text(source)),
                                deps: refs.names,
                                writes: refs.writes,
                            });
                        }
                    }
                }
                Statement::EmptyStatement(_) => {}
                _ => {
                    let mut refs = References {
                        scoping: scopes,
                        names: BTreeSet::new(),
                        writes: BTreeSet::new(),
                    };
                    refs.visit_statement(statement);
                    if !refs.writes.is_empty() {
                        module.fragments.push(Fragment {
                            span: statement.span(),
                            bridge: None,
                            code: statement.span().source_text(source).into(),
                            deps: refs.names,
                            writes: refs.writes,
                        });
                    }
                }
            }
        }
        module.fragments.sort_by_key(|f| f.span.start);
        for (index, fragment) in module.fragments.iter().enumerate() {
            for name in &fragment.writes {
                module
                    .mutations
                    .entry(name.clone())
                    .or_default()
                    .push(index);
            }
        }
        for symbol in scopes.symbol_ids() {
            if scopes.symbol_scope_id(symbol) != scopes.root_scope_id() {
                continue;
            }
            let name = scopes.symbol_name(symbol);
            if module.imports.contains_key(name) {
                continue;
            }
            let span = scopes.symbol_span(symbol);
            let index = module
                .fragments
                .partition_point(|f| f.span.end <= span.start);
            if let Some(fragment) = module.fragments.get(index)
                && fragment.span.start <= span.start
                && span.end <= fragment.span.end
            {
                module.bindings.insert(name.into(), index);
            }
        }
        let mut meta = Metadata {
            keys: BTreeMap::new(),
            classes: BTreeSet::new(),
        };
        meta.visit_program(&parsed.program);
        module.loader_keys = meta.keys;
        module.class_names = meta.classes;
        let getters = module
            .fragments
            .iter()
            .filter(|f| {
                matches!(
                    f.bridge,
                    Some(Bridge::Router("useLocation" | "useRouterState"))
                )
            })
            .flat_map(|f| f.deps.iter())
            .filter_map(|name| module.bindings.get(name))
            .copied()
            .collect::<BTreeSet<_>>();
        let context_getter = regex::Regex::new(
            r"^function\s+[\w$]+\([^)]*\)\{return\s+[\w$]+\.useContext\([\w$]+\);?\}$",
        )?;
        for index in getters {
            if context_getter.is_match(&module.fragments[index].code) {
                module.fragments[index].bridge = Some(Bridge::Router("useRouter"));
            }
        }
        ensure!(module.fragments.len() <= 100_000, "Vite declaration limit");
        Ok(module)
    }
}

struct Registry<'r> {
    root: &'r Path,
    budget: u64,
    modules: BTreeMap<String, Module>,
    sources: BTreeMap<String, String>,
    selected: BTreeMap<String, BTreeSet<usize>>,
    requested: BTreeMap<String, BTreeSet<String>>,
    active_catalog: Vec<Literal>,
    active_data: BTreeMap<String, Literal>,
    visited: BTreeSet<(String, String)>,
    depth: usize,
}
impl Registry<'_> {
    fn load(&mut self, url: &str) -> Result<()> {
        if self.modules.contains_key(url) {
            return Ok(());
        }
        let parsed = Url::parse(url)?;
        ensure!(
            parsed.query().is_none() && parsed.fragment().is_none(),
            "ambiguous source module URL"
        );
        let file = mirror::output_path(&self.root.join("site"), &parsed, false)?;
        let source = read(self.root, &file, &mut self.budget)
            .with_context(|| format!("read source module {url}"))?;
        let module = Module::parse(&source).with_context(|| format!("parse {url}"))?;
        self.sources.insert(
            file.strip_prefix(self.root)?.to_string_lossy().into_owned(),
            fingerprint(source.as_bytes()),
        );
        self.modules.insert(url.into(), module);
        Ok(())
    }
    fn resolve(&self, base: &str, relative: &str) -> Result<String> {
        let base = Url::parse(base)?;
        let url = base.join(relative)?;
        ensure!(
            url.origin() == base.origin(),
            "external module requires a package adapter"
        );
        Ok(url.to_string())
    }
    fn select(&mut self, url: &str, export: &str) -> Result<()> {
        self.load(url)?;
        self.requested
            .entry(url.into())
            .or_default()
            .insert(export.into());
        let local = self.modules[url]
            .exports
            .get(export)
            .with_context(|| format!("missing Vite export {export} in {url}"))?
            .clone();
        self.select_binding(url, &local)
    }
    fn select_binding(&mut self, url: &str, local: &str) -> Result<()> {
        ensure!(self.depth < 128, "Vite dependency depth exceeded");
        self.depth += 1;
        let result = self.select_binding_inner(url, local);
        self.depth -= 1;
        result
    }
    fn select_binding_inner(&mut self, url: &str, local: &str) -> Result<()> {
        if !self.visited.insert((url.into(), local.into())) {
            return Ok(());
        }
        if let Some(import) = self.modules[url].imports.get(local).cloned() {
            let target = self.resolve(url, &import.source)?;
            if let Some(keys) = self.modules[url].loader_keys.get(local).cloned() {
                self.load(&target)?;
                let binding = self.modules[&target]
                    .exports
                    .get(&import.name)
                    .context("missing route metadata export")?
                    .clone();
                let index = self.modules[&target].bindings[&binding];
                let key = fingerprint(format!("{target}#{}", import.name).as_bytes());
                let value = data(&self.active_catalog, &keys)?;
                self.active_data.insert(key.clone(), value);
                self.modules
                    .get_mut(&target)
                    .context("missing route metadata")?
                    .fragments[index]
                    .bridge = Some(Bridge::Data(key));
            }
            return self.select(&target, &import.name);
        }
        let index = *self.modules[url]
            .bindings
            .get(local)
            .with_context(|| format!("unresolved source binding {local} in {url}"))?;
        self.selected.entry(url.into()).or_default().insert(index);
        let fragment = self.modules[url].fragments[index].clone();
        if fragment.bridge.is_none() {
            for dependency in fragment.deps {
                if dependency != local {
                    self.select_binding(url, &dependency)?;
                }
            }
        }
        let mutations = self.modules[url]
            .mutations
            .get(local)
            .into_iter()
            .flatten()
            .map(|&i| (i, self.modules[url].fragments[i].deps.clone()))
            .collect::<Vec<_>>();
        for (i, deps) in mutations {
            self.selected
                .get_mut(url)
                .context("missing selection")?
                .insert(i);
            for dep in deps {
                self.select_binding(url, &dep)?;
            }
        }
        ensure!(self.selected.len() <= 128, "Vite module graph limit");
        Ok(())
    }
}
fn filename(url: &str) -> String {
    format!("Esm{}", &fingerprint(url.as_bytes())[..16])
}
fn manifest(catalog: &[Literal]) -> Result<&Literal> {
    let values = catalog
        .iter()
        .filter_map(|v| v.get("routes"))
        .filter(|v| v.get("__root__").is_some())
        .collect::<Vec<_>>();
    ensure!(
        values.len() == 1,
        "missing or ambiguous Vite route manifest"
    );
    Ok(values[0])
}
fn public_path(id: &str) -> String {
    let segments = id
        .split('/')
        .filter(|s| !s.is_empty() && !s.starts_with('_'))
        .collect::<Vec<_>>();
    format!("/{}", segments.join("/"))
}
fn component(registry: &mut Registry<'_>, base: &str, route: &Literal) -> Result<String> {
    let Some(Literal::Array(preloads)) = route.get("preloads") else {
        anyhow::bail!("route manifest lacks preloads")
    };
    let mut candidates = Vec::new();
    for preload in preloads {
        let Some(preload) = preload.string() else {
            continue;
        };
        if !preload.ends_with(".js") {
            continue;
        }
        let url = registry.resolve(base, preload)?;
        registry.load(&url)?;
        if registry.modules[&url].exports.contains_key("component") {
            candidates.push(url);
        }
    }
    ensure!(
        candidates.len() == 1,
        "expected one split route component, found {}",
        candidates.len()
    );
    Ok(candidates.remove(0))
}
fn bridge_source(bridge: &Bridge, names: &[String]) -> Result<String> {
    ensure!(
        names.len() == 1,
        "bridged declaration has multiple bindings"
    );
    let name = &names[0];
    Ok(match bridge {
        Bridge::React => format!("function {name}(){{return __mochiReact}}"),
        Bridge::Jsx => format!("function {name}(){{return __mochiJsx}}"),
        Bridge::Compiler => format!("function {name}(){{return __mochiCompiler}}"),
        Bridge::Dom => format!("function {name}(){{return __mochiDom}}"),
        Bridge::Router(api) => {
            format!("function {name}(...args){{return __mochiRouter.{api}(...args)}}")
        }
        Bridge::Outlet => format!("const {name}=__mochiReact.memo(__mochiOutlet);"),
        Bridge::Data(key) => format!(
            "const {name}={{useLoaderData(options){{return __mochiData({},options)}}}};",
            serde_json::to_string(key)?
        ),
    })
}
const HOST: &str = r#"import {createContext,useContext} from 'react'
export const LoaderContext=createContext({})
export const OutletContext=createContext(null)
export function SourceOutlet(){return useContext(OutletContext)}
export function useCapturedData(key,options){const values=useContext(LoaderContext);if(!Object.hasOwn(values,key))throw Error('Missing captured route data');const value=values[key];return options?.select?options.select(value):value}
"#;
pub(super) fn plan(root: &Path, model: &Project, paths: &BTreeSet<String>) -> Result<Recovery> {
    let mut registry = Registry {
        root,
        budget: 0,
        modules: BTreeMap::new(),
        sources: BTreeMap::new(),
        selected: BTreeMap::new(),
        requested: BTreeMap::new(),
        active_catalog: Vec::new(),
        active_data: BTreeMap::new(),
        visited: BTreeSet::new(),
        depth: 0,
    };
    let mut recovery = Recovery::default();
    recovery.add("src/compat/recovered/host.js".into(), HOST.into())?;
    recovery.add("src/compat/recovered/host.d.ts".into(),"import type {Context,ReactNode} from 'react';export declare const LoaderContext:Context<Record<string,unknown>>;export declare const OutletContext:Context<ReactNode>;export declare function SourceOutlet():ReactNode;export declare function useCapturedData(key:string,options?:{select?:(data:unknown)=>unknown}):unknown;\n".into())?;
    for path in paths {
        let routes = model
            .routes
            .values()
            .filter(|r| &r.pathname == path)
            .collect::<Vec<_>>();
        ensure!(
            routes.len() == 1,
            "Vite recovery requires one literal route state"
        );
        let route = routes[0];
        let base = Url::parse(&route.url)?;
        let html_path = mirror::output_path(&root.join("site"), &base, true)?;
        let html = read(root, &html_path, &mut registry.budget)?;
        registry.sources.insert(
            html_path.strip_prefix(root)?.to_string_lossy().into_owned(),
            fingerprint(html.as_bytes()),
        );
        let values = catalog(&html)?;
        let manifest = manifest(&values)?;
        let Literal::Object(entries) = manifest else {
            anyhow::bail!("invalid route manifest")
        };
        let mut matches = entries
            .iter()
            .filter(|(id, _)| id != "__root__" && public_path(id) == *path)
            .collect::<Vec<_>>();
        matches.sort_by_key(|(id, _)| id.len());
        let (id, leaf) = matches
            .last()
            .context("source manifest has no matching literal route")?;
        let leaf = component(&mut registry, base.as_str(), leaf)?;
        let mut parents = entries
            .iter()
            .filter(|(parent, _)| {
                parent != "__root__"
                    && parent != id
                    && id.starts_with(&format!("{}/", parent.trim_end_matches('/')))
                    && public_path(parent) == "/"
            })
            .collect::<Vec<_>>();
        parents.sort_by_key(|(id, _)| id.len());
        let mut layouts = Vec::new();
        for (_, route) in parents {
            layouts.push(component(&mut registry, base.as_str(), route)?);
        }
        registry.active_catalog = values;
        registry.active_data.clear();
        registry.visited.clear();
        registry.select(&leaf, "component")?;
        for layout in &layouts {
            registry.select(layout, "component")?;
        }
        let wrapper = format!("Client{}", &fingerprint(route.url.as_bytes())[..16]);
        let mut source = format!(
            "import {{LoaderContext,OutletContext}} from '../compat/recovered/host.js'\nimport {{component as Page}} from '../compat/recovered/{}.js'\n",
            filename(&leaf)
        );
        for (i, layout) in layouts.iter().enumerate() {
            source.push_str(&format!(
                "import {{component as Layout{i}}} from '../compat/recovered/{}.js'\n",
                filename(layout)
            ));
        }
        let data = Literal::Object(
            registry
                .active_data
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
        .source()?;
        source.push_str(&format!(
            "const data={data}\nexport function {wrapper}(){{return <LoaderContext value={{data}}>"
        ));
        let mut node = "<Page />".to_owned();
        for i in (0..layouts.len()).rev() {
            node = format!("<OutletContext value={{{node}}}><Layout{i} /></OutletContext>");
        }
        source.push_str(&format!("{node}</LoaderContext>}}\n"));
        recovery.add(format!("src/components/{wrapper}.tsx"), source)?;
        recovery.routes.insert(
            path.clone(),
            RecoveredRoute {
                component: wrapper,
                module: 0,
                source: leaf,
                method: "vite_esm_binding_recovery".into(),
            },
        );
    }
    for (url, selected) in &registry.selected {
        let module = &registry.modules[url];
        let mut source = String::from(
            "import * as __mochiReact from 'react';\nimport * as __mochiJsx from 'react/jsx-runtime';\nimport * as __mochiCompiler from 'react/compiler-runtime';\nimport * as __mochiDom from 'react-dom';\nimport * as __mochiRouter from '@tanstack/react-router';\nimport {SourceOutlet as __mochiOutlet,useCapturedData as __mochiData} from './host.js';\n",
        );
        let mut needed = BTreeSet::new();
        for index in selected {
            if module.fragments[*index].bridge.is_none() {
                needed.extend(module.fragments[*index].deps.iter().cloned());
            }
        }
        for (local, import) in &module.imports {
            if needed.contains(local) {
                let target = registry.resolve(url, &import.source)?;
                ensure!(
                    registry
                        .requested
                        .get(&target)
                        .is_some_and(|names| names.contains(&import.name)),
                    "unselected dependency export"
                );
                source.push_str(&format!(
                    "import {{{} as {local}}} from './{}.js';\n",
                    serde_json::to_string(&import.name)?,
                    filename(&target)
                ));
            }
        }
        for index in selected {
            let fragment = &module.fragments[*index];
            if let Some(bridge) = &fragment.bridge {
                let names = module
                    .bindings
                    .iter()
                    .filter(|(_, i)| *i == index)
                    .map(|(n, _)| n.clone())
                    .collect::<Vec<_>>();
                source.push_str(&bridge_source(bridge, &names)?);
            } else {
                source.push_str(&fragment.code);
            }
            source.push('\n');
        }
        let exports = &registry.requested[url];
        for export in exports {
            let local = &module.exports[export];
            source.push_str(&format!(
                "export {{{local} as {}}};\n",
                serde_json::to_string(export)?
            ));
        }
        let header = format!(
            "// Recovered Vite binding slice from {}; unrelated app bootstrap excluded.\n",
            serde_json::to_string(url)?
        );
        recovery.add(
            format!("src/compat/recovered/{}.js", filename(url)),
            header + &turbopack::format(&source)?,
        )?;
        let mut declaration = exports
            .iter()
            .map(|name| module.exports[name].clone())
            .collect::<BTreeSet<_>>()
            .iter()
            .map(|local| format!("declare const {local}:any;\n"))
            .collect::<String>();
        for name in exports {
            let local = &module.exports[name];
            declaration.push_str(&format!(
                "export {{{local} as {}}};\n",
                serde_json::to_string(name)?
            ));
        }
        recovery.add(
            format!("src/compat/recovered/{}.d.ts", filename(url)),
            declaration,
        )?;
    }
    ensure!(
        registry
            .selected
            .iter()
            .any(|(url, indices)| indices.iter().any(|index| matches!(
                registry.modules[url].fragments[*index].bridge,
                Some(Bridge::React)
            ))),
        "Vite recovery needs a supported React 19.2.7 factory"
    );
    recovery.sources = registry.sources;
    recovery.react_version = Some("19.2.7".into());
    Ok(recovery)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(root: &Path) -> Registry<'_> {
        Registry {
            root,
            budget: 0,
            modules: BTreeMap::new(),
            sources: BTreeMap::new(),
            selected: BTreeMap::new(),
            requested: BTreeMap::new(),
            active_catalog: Vec::new(),
            active_data: BTreeMap::new(),
            visited: BTreeSet::new(),
            depth: 0,
        }
    }

    #[test]
    fn router_data_preserves_undefined_references_and_refuses_executable_values() {
        let values = catalog(r#"<script data-tsr-stream-part>($R[0]={title:'Captured',optional:void 0,on:!0,off:!1,n:-3});$R[1]={data:$R[0],items:[$R[0]],unsafe:fetch('/must-not-run')};</script>"#).unwrap();
        let data = data(
            &values,
            &BTreeSet::from(["title".into(), "optional".into()]),
        )
        .unwrap();
        assert_eq!(data.get("optional"), Some(&Literal::Undefined));
        assert_eq!(data.get("on"), Some(&Literal::Json(Value::Bool(true))));
        assert_eq!(data.get("off"), Some(&Literal::Json(Value::Bool(false))));
        assert_eq!(data.get("n"), Some(&Literal::Json(serde_json::json!(-3.0))));
        assert!(data.source().unwrap().contains("void 0"));
        assert!(values.iter().all(|v| v.get("unsafe").is_none()));
        let ambiguous =
            catalog("<script data-tsr-stream-part>({title:'one'});({title:'two'})</script>")
                .unwrap();
        assert!(super::data(&ambiguous, &BTreeSet::from(["title".into()])).is_err());
        let cycle = catalog("<script data-tsr-stream-part>$R[0]={self:$R[0]}</script>").unwrap();
        assert!(cycle.is_empty());
        let mut expanded = String::from("<script data-tsr-stream-part>$R[0]={leaf:1};");
        for i in 1..25 {
            expanded.push_str(&format!("$R[{i}]={{a:$R[{}],b:$R[{}]}};", i - 1, i - 1));
        }
        expanded.push_str("</script>");
        assert!(
            catalog(&expanded)
                .unwrap_err()
                .to_string()
                .contains("expansion budget")
        );
    }

    #[test]
    fn binding_slice_keeps_transitive_imports_and_prototype_writes_without_bootstrap() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("site")).unwrap();
        std::fs::write(
            temp.path().join("site/dependency.js"),
            "const value=41; export {value as v};",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("site/page.js"),
            r#"
            import {v as value} from './dependency.js';
            const unrelated=()=>{throw Error('BOOTSTRAP_MUST_NOT_RUN')};
            class Scene { render(value) {return value;} }
            Scene.prototype.answer=function(){return value+1};
            Object.defineProperty(Scene,'displayName',{value:'LiveScene'});
            function component(){return new Scene().answer()}
            unrelated(); export {component};
        "#,
        )
        .unwrap();
        let mut registry = registry(temp.path());
        let url = "https://example.test/page.js";
        registry.select(url, "component").unwrap();
        let selected = registry.selected[url]
            .iter()
            .map(|&i| registry.modules[url].fragments[i].code.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(selected.contains("Scene.prototype.answer"));
        assert!(selected.contains("Object.defineProperty"));
        assert!(!selected.contains("BOOTSTRAP_MUST_NOT_RUN"));
        assert!(registry.requested["https://example.test/dependency.js"].contains("v"));
        let scene = &registry.modules[url].fragments[registry.modules[url].bindings["Scene"]];
        assert!(
            !scene.deps.contains("value"),
            "method parameter shadows imported value"
        );
        assert_eq!(registry.sources.len(), 2);
    }

    #[test]
    fn binding_walk_refuses_excessive_depth_and_missing_exports() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("site")).unwrap();
        let mut source = String::from("const x0=0;\n");
        for i in 1..150 {
            source.push_str(&format!("const x{i}=x{}+1;\n", i - 1));
        }
        source.push_str("export {x149 as component};");
        std::fs::write(temp.path().join("site/page.js"), source).unwrap();
        let mut registry = registry(temp.path());
        assert!(
            registry
                .select("https://example.test/page.js", "component")
                .unwrap_err()
                .to_string()
                .contains("depth")
        );
        assert!(
            registry
                .select("https://example.test/page.js", "absent")
                .is_err()
        );
        assert_eq!(registry.depth, 0);
    }
}
