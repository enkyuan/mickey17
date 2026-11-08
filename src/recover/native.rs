//! Native custom-element modules plus pristine server markup. Unlike the React
//! adapter, this preserves module initialization as well as declarations. It does
//! not interpret an opaque runtime or execute source during offline generation.
use super::*;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};
use scraper::{ElementRef, Html, Selector};

fn html(
    root: &Path,
    url: &str,
    budget: &mut u64,
    sources: &mut BTreeMap<String, String>,
) -> Result<String> {
    let path = mirror::output_path(&root.join("site"), &Url::parse(url)?, true)?;
    let source = read(root, &path, budget)?;
    sources.insert(
        path.strip_prefix(root)?.to_string_lossy().into_owned(),
        fingerprint(source.as_bytes()),
    );
    Ok(source)
}
fn entries(source: &str, base: &Url) -> Result<Vec<Url>> {
    let doc = Html::parse_document(source);
    let selector = Selector::parse("script[type=module][src]")
        .map_err(|_| anyhow::anyhow!("module selector"))?;
    let mut entries = Vec::new();
    for script in doc.select(&selector) {
        let url = base.join(
            script
                .value()
                .attr("src")
                .context("missing script source")?,
        )?;
        if url.origin() == base.origin() {
            entries.push(url);
        }
    }
    Ok(entries)
}
fn custom_registration(source: &str) -> bool {
    source.contains("customElements.define(")
}
pub(super) fn candidate(root: &Path, model: &Project, paths: &BTreeSet<String>) -> Result<bool> {
    let mut budget = 0;
    for route in model
        .routes
        .values()
        .filter(|r| paths.contains(&r.pathname))
    {
        let base = Url::parse(&route.url)?;
        let source = html(root, &route.url, &mut budget, &mut BTreeMap::new())?;
        if source.contains("data-tsr-stream-part") {
            return Ok(false);
        }
        for entry in entries(&source, &base)? {
            let path = mirror::output_path(&root.join("site"), &entry, false)?;
            let script = read(root, &path, &mut budget)?;
            if custom_registration(&script) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
#[derive(Default)]
struct Imports {
    items: Vec<(Span, String)>,
    dynamic: Vec<(Span, Span)>,
    registered: BTreeSet<String>,
    invalid: bool,
}
fn text(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::StringLiteral(s) => s.value.as_str().map(str::to_owned),
        Expression::TemplateLiteral(t) if t.expressions.is_empty() => t
            .quasis
            .first()?
            .value
            .cooked
            .as_ref()?
            .as_str()
            .map(str::to_owned),
        _ => None,
    }
}
impl<'a> Visit<'a> for Imports {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Expression::StaticMemberExpression(member) = &call.callee
            && member.property.name == "define"
            && let Expression::Identifier(object) = &member.object
            && object.name == "customElements"
            && let Some(name) = call
                .arguments
                .first()
                .and_then(Argument::as_expression)
                .and_then(text)
        {
            self.registered.insert(name);
        }
        walk::walk_call_expression(self, call);
    }
    fn visit_import_declaration(&mut self, import: &ImportDeclaration<'a>) {
        if let Some(value) = import.source.value.as_str() {
            self.items.push((import.source.span, value.into()))
        } else {
            self.invalid = true
        }
    }
    fn visit_export_from_declaration(&mut self, export: &ExportFromDeclaration<'a>) {
        if let Some(value) = export.source.value.as_str() {
            self.items.push((export.source.span, value.into()))
        } else {
            self.invalid = true
        }
    }
    fn visit_export_all_declaration(&mut self, export: &ExportAllDeclaration<'a>) {
        if let Some(value) = export.source.value.as_str() {
            self.items.push((export.source.span, value.into()))
        } else {
            self.invalid = true
        }
    }
    fn visit_import_expression(&mut self, import: &ImportExpression<'a>) {
        if let Some(value) = text(&import.source) {
            self.items.push((import.source.span(), value))
        } else {
            self.dynamic.push((import.span, import.source.span()));
        }
        walk::walk_import_expression(self, import);
    }
}
fn filename(url: &Url) -> String {
    format!("Native{}", &fingerprint(url.as_str().as_bytes())[..16])
}
pub(super) fn literal_references(source: &str, base: &Url) -> BTreeSet<Url> {
    if source.len() > FILE_LIMIT as usize {
        return BTreeSet::new();
    }
    let arena = Allocator::default();
    let parsed = Parser::new(&arena, source, SourceType::mjs()).parse();
    if !parsed.diagnostics.is_empty() || turbopack::format_shape(&parsed.program).is_err() {
        return BTreeSet::new();
    }
    let mut imports = Imports::default();
    imports.visit_program(&parsed.program);
    imports
        .items
        .into_iter()
        .filter_map(|(_, relative)| {
            if !(relative.starts_with('.') || relative.starts_with('/')) {
                return None;
            }
            base.join(&relative)
                .ok()
                .filter(|u| u.origin() == base.origin() && matches!(u.scheme(), "https" | "http"))
        })
        .collect()
}
struct Graph<'r> {
    root: &'r Path,
    budget: u64,
    seen: BTreeSet<String>,
    dynamic: BTreeMap<String, Url>,
    registered: BTreeSet<String>,
    recovery: Recovery,
}
impl Graph<'_> {
    fn module(&mut self, url: &Url, depth: usize) -> Result<String> {
        ensure!(depth < 128, "native dependency depth exceeded");
        ensure!(url.fragment().is_none(), "ambiguous native module URL");
        let name = filename(url);
        if !self.seen.insert(url.to_string()) {
            return Ok(name);
        }
        ensure!(self.seen.len() <= 128, "native module count exceeded");
        let exact = self
            .root
            .join("site/.mochi-source")
            .join(format!("{}.blob", fingerprint(url.as_str().as_bytes())));
        let path = if exact.is_file() {
            exact
        } else {
            ensure!(
                url.query().is_none(),
                "query module requires an exact full-URL response receipt"
            );
            mirror::output_path(&self.root.join("site"), url, false)?
        };
        let source = read(self.root, &path, &mut self.budget)
            .with_context(|| format!("read native module {url}"))?;
        self.recovery.sources.insert(
            path.strip_prefix(self.root)?.to_string_lossy().into_owned(),
            fingerprint(source.as_bytes()),
        );
        let arena = Allocator::default();
        let parsed = Parser::new(&arena, &source, SourceType::mjs()).parse();
        ensure!(parsed.diagnostics.is_empty(), "invalid native module {url}");
        turbopack::format_shape(&parsed.program)?;
        let mut imports = Imports::default();
        imports.visit_program(&parsed.program);
        self.registered.extend(imports.registered);
        ensure!(
            !imports.invalid,
            "nonliteral native module import; explicit package adapter required"
        );
        let mut replacements = Vec::new();
        let mut dispatch = String::new();
        if !imports.dynamic.is_empty() {
            ensure!(
                !self.dynamic.is_empty(),
                "nonliteral native module import lacks captured markup targets"
            );
            ensure!(
                !source.contains("__mochiImportRecorded"),
                "native helper collision"
            );
            dispatch.push_str("function __mochiImportRecorded(specifier){switch(specifier){");
            for (literal, target) in self.dynamic.clone() {
                let target = self.module(&target, depth + 1)?;
                dispatch.push_str(&format!(
                    "case {}:return import('./{target}.js');",
                    serde_json::to_string(&literal)?
                ));
            }
            dispatch.push_str(
                "default:return Promise.reject(Error('Uncaptured native module: '+specifier));}}\n",
            );
            for (span, arg) in imports.dynamic {
                replacements.push((
                    span,
                    format!("__mochiImportRecorded({})", arg.source_text(&source)),
                ));
            }
        }
        for (span, relative) in imports.items {
            ensure!(
                relative.starts_with('.') || relative.starts_with('/'),
                "bare or external native module import"
            );
            let target = url.join(&relative)?;
            ensure!(
                target.origin() == url.origin(),
                "cross-origin module graph requires a package adapter"
            );
            let target = self.module(&target, depth + 1)?;
            replacements.push((span, serde_json::to_string(&format!("./{target}.js"))?));
        }
        replacements.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
        let mut source = source;
        for (span, replacement) in replacements {
            source.replace_range(span.start as usize..span.end as usize, &replacement)
        }
        let source = format!(
            "// Recovered native ESM from {}. Module initialization is retained.\n{}",
            serde_json::to_string(url.as_str())?,
            turbopack::format(&(dispatch + &source))?
        );
        self.recovery
            .add(format!("src/compat/recovered/{name}.js"), source)?;
        self.recovery.add(
            format!("src/compat/recovered/{name}.d.ts"),
            "export {};\n".into(),
        )?;
        Ok(name)
    }
}
struct Markup {
    bytes: usize,
    nodes: usize,
    custom: BTreeSet<String>,
    omitted: BTreeSet<String>,
}
impl Markup {
    fn element(&mut self, element: ElementRef<'_>, depth: usize) -> Result<String> {
        ensure!(
            depth < 128 && self.nodes < 20_000,
            "native markup shape exceeded"
        );
        self.nodes += 1;
        let tag = element.value().name();
        if self.omitted.contains(tag) {
            return Ok(String::new());
        }
        if matches!(
            tag,
            "script"
                | "style"
                | "link"
                | "meta"
                | "base"
                | "noscript"
                | "iframe"
                | "object"
                | "embed"
        ) {
            return Ok(String::new());
        }
        ensure!(
            tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "unsupported native markup tag"
        );
        if tag.contains('-') {
            self.custom.insert(tag.into());
        }
        let attrs = element
            .value()
            .attrs()
            .filter(|(name, _)| {
                !name.to_ascii_lowercase().starts_with("on")
                    && !matches!(*name, "action" | "formaction" | "srcdoc" | "is" | "srcset")
            })
            .collect::<BTreeMap<_, _>>();
        let mut source = format!(
            "<{tag} {{...props({}, {})}}",
            serde_json::to_string(tag)?,
            serde_json::to_string(&attrs)?
        );
        let voids = "area base br col embed hr img input link meta param source track wbr";
        if voids.split_whitespace().any(|v| v == tag) {
            source.push_str(" />");
            return Ok(source);
        }
        source.push('>');
        for child in element.children() {
            if let Some(element) = ElementRef::wrap(child) {
                source.push_str(&self.element(element, depth + 1)?)
            } else if let scraper::Node::Text(text) = child.value() {
                source.push_str(&format!("{{{}}}", serde_json::to_string(&**text)?));
                self.nodes += 1;
            }
        }
        source.push_str(&format!("</{tag}>"));
        self.bytes += source.len();
        ensure!(
            self.bytes <= 128 * 1024 * 1024,
            "native markup output budget exceeded"
        );
        Ok(source)
    }
}
pub(super) fn plan(root: &Path, model: &Project, paths: &BTreeSet<String>) -> Result<Recovery> {
    let mut graph = Graph {
        root,
        budget: 0,
        seen: BTreeSet::new(),
        dynamic: BTreeMap::new(),
        registered: BTreeSet::new(),
        recovery: Recovery::default(),
    };
    let mut markup = Markup {
        bytes: 0,
        nodes: 0,
        custom: BTreeSet::new(),
        omitted: BTreeSet::new(),
    };
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let manifest = root.join("manifest.json");
    if manifest.is_file() {
        let source = read(root, &manifest, &mut graph.budget)?;
        let value: Value = serde_json::from_str(&source)?;
        for page in value["pages"].as_array().into_iter().flatten() {
            if let (Some(from), Some(to)) = (page["url"].as_str(), page["final_url"].as_str())
                && from != to
                && model.routes.values().any(|r| r.url == to)
            {
                aliases.insert(from.into(), to.into());
            }
        }
        graph
            .recovery
            .sources
            .insert("manifest.json".into(), fingerprint(source.as_bytes()));
    }
    let mut pages = Vec::new();
    for path in paths {
        let routes = model
            .routes
            .values()
            .filter(|r| &r.pathname == path)
            .collect::<Vec<_>>();
        ensure!(
            routes.len() == 1,
            "native recovery needs one literal route state"
        );
        let route = routes[0];
        let base = Url::parse(&route.url)?;
        let source = html(
            root,
            &route.url,
            &mut graph.budget,
            &mut graph.recovery.sources,
        )?;
        let doc = Html::parse_document(&source);
        for node in doc.tree.nodes() {
            if let scraper::Node::Element(element) = node.value() {
                for (name, value) in element.attrs() {
                    if name.starts_with("data-")
                        && let Ok(url) = base.join(value)
                        && url.path().ends_with(".js")
                        && url.origin() == base.origin()
                    {
                        graph.dynamic.insert(value.to_owned(), url);
                        ensure!(
                            graph.dynamic.len() <= 128,
                            "markup module target limit exceeded"
                        );
                    }
                }
            }
        }
        pages.push((path, route, base, source));
    }
    for (path, route, base, source) in pages {
        let mut clients = Vec::new();
        for url in entries(&source, &base)? {
            clients.push(graph.module(&url, 0)?)
        }
        ensure!(
            !clients.is_empty(),
            "native recovery lacks a same-origin module entry"
        );
        // External platform elements without recovered controllers can otherwise
        // expose hidden light-DOM scaffolding. Omit only tags consistently
        // nonvisual in every selected observation; keep registered hidden UI.
        let mut visibility: BTreeMap<String, bool> = BTreeMap::new();
        for observation in model
            .observations
            .values()
            .filter(|o| paths.contains(&model.routes[&o.route].pathname))
        {
            for node in &observation.capture.nodes {
                if let crate::evidence::Content::Element { tag, bounds, .. } = &node.content
                    && tag.contains('-')
                    && !graph.registered.contains(tag)
                {
                    *visibility.entry(tag.clone()).or_default() |=
                        bounds.width >= 0.5 && bounds.height >= 0.5;
                }
            }
        }
        markup.omitted = visibility
            .into_iter()
            .filter_map(|(tag, visible)| (!visible).then_some(tag))
            .collect();
        let doc = Html::parse_document(&source);
        let selector = Selector::parse("body").map_err(|_| anyhow::anyhow!("body selector"))?;
        let body = doc
            .select(&selector)
            .next()
            .context("native source body missing")?;
        let mut jsx = String::new();
        for child in body.children() {
            if let Some(element) = ElementRef::wrap(child) {
                jsx.push_str(&markup.element(element, 0)?);
            }
        }
        let name = format!("Client{}", &fingerprint(route.url.as_bytes())[..16]);
        let imports = clients
            .iter()
            .map(|name| format!("await import('../compat/recovered/{name}.js');"))
            .collect::<String>();
        let component = format!(
            "import {{memo,useEffect,type JSX}} from 'react';\nimport {{elementProps}} from '../lib/runtime';\nconst base={};\nconst aliases:Record<string,string>={};\nfunction props<T extends keyof JSX.IntrinsicElements>(tag:T,raw:Record<string,string>){{if(tag==='a'&&raw.href){{const url=new URL(raw.href,base);const fragment=url.hash;url.hash='';if(aliases[url.href])raw={{...raw,href:aliases[url.href]+fragment}}}}return elementProps(tag,raw,null,base)}}\nexport const {name}=memo(function {name}(){{useEffect(()=>{{let disposed=false;const start=async()=>{{if(document.readyState!=='complete')await new Promise<void>(resolve=>window.addEventListener('load',()=>resolve(),{{once:true}}));if(disposed)return;{imports}}};void start().catch(error=>{{console.error('Recovered native initialization failed',error);document.documentElement.dataset.mochiClientError=String(error)}});return()=>{{disposed=true}}}},[]);return <>{jsx}</>}});\n",
            serde_json::to_string(base.as_str())?,
            serde_json::to_string(&aliases)?
        );
        graph
            .recovery
            .add(format!("src/components/{name}.tsx"), component)?;
        graph.recovery.routes.insert(
            path.clone(),
            RecoveredRoute {
                component: name,
                module: 0,
                source: base.to_string(),
                method: "native_custom_elements_esm".into(),
            },
        );
    }
    ensure!(
        !markup.custom.is_empty(),
        "native adapter requires custom elements"
    );
    if !markup.omitted.is_empty() {
        graph.recovery.notes.push(format!("Omitted external custom elements consistently nonvisual in all selected observations: {}",markup.omitted.iter().cloned().collect::<Vec<_>>().join(", ")));
    }
    let declarations = markup
        .custom
        .iter()
        .map(|name| {
            format!(
                "'{}':HTMLAttributes<HTMLElement>&Record<string,unknown>;",
                name
            )
        })
        .collect::<String>();
    graph.recovery.add("src/recovered-elements.d.ts".into(),format!("import type {{HTMLAttributes}} from 'react';\ndeclare module 'react'{{namespace JSX{{interface IntrinsicElements{{{declarations}}}}}}}\n"))?;
    Ok(graph.recovery)
}
