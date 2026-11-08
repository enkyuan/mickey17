//! Reviewed typed algorithms are ordinary source files, not opaque recovered chunks.
use super::*;
use anyhow::{Context, Result, ensure};
use oxc_ast::ast;
use oxc_ast_visit::{Visit, walk};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) type Files = BTreeMap<String, String>;
#[derive(Default)]
struct Imports {
    imports: BTreeSet<String>,
    exports: BTreeSet<String>,
    errors: Vec<&'static str>,
}
impl<'a> Visit<'a> for Imports {
    fn visit_import_declaration(&mut self, n: &ast::ImportDeclaration<'a>) {
        if let Some(s) = n.source.value.as_str() {
            self.imports.insert(s.into());
        } else {
            self.errors.push("invalid import");
        }
        walk::walk_import_declaration(self, n);
    }
    fn visit_import_expression(&mut self, n: &ast::ImportExpression<'a>) {
        if let ast::Expression::StringLiteral(s) = &n.source {
            if let Some(s) = s.value.as_str() {
                self.imports.insert(s.into());
            }
        } else {
            self.errors
                .push("dynamic import must have a literal module path");
        }
        walk::walk_import_expression(self, n);
    }
    fn visit_export_all_declaration(&mut self, n: &ast::ExportAllDeclaration<'a>) {
        if let Some(s) = n.source.value.as_str() {
            self.imports.insert(s.into());
        }
        walk::walk_export_all_declaration(self, n);
    }
    fn visit_export_declaration(&mut self, n: &ast::ExportDeclaration<'a>) {
        if let ast::Declaration::FunctionDeclaration(f) = &n.declaration
            && let Some(id) = &f.id
        {
            self.exports.insert(id.name.to_string());
        }
        if let ast::Declaration::VariableDeclaration(v) = &n.declaration {
            for d in &v.declarations {
                if let ast::BindingPattern::BindingIdentifier(id) = &d.id {
                    self.exports.insert(id.name.to_string());
                }
            }
        }
        walk::walk_export_declaration(self, n);
    }
    fn visit_export_from_declaration(&mut self, n: &ast::ExportFromDeclaration<'a>) {
        if let Some(s) = n.source.value.as_str() {
            self.imports.insert(s.into());
        }
        walk::walk_export_from_declaration(self, n);
    }
    fn visit_export_named_declaration(&mut self, n: &ast::ExportNamedDeclaration<'a>) {
        for spec in &n.specifiers {
            match &spec.exported {
                ast::ModuleExportName::IdentifierName(id) => {
                    self.exports.insert(id.name.to_string());
                }
                ast::ModuleExportName::IdentifierReference(id) => {
                    self.exports.insert(id.name.to_string());
                }
                _ => self.errors.push("owned exports require identifiers"),
            }
        }
        walk::walk_export_named_declaration(self, n);
    }
    fn visit_call_expression(&mut self, n: &ast::CallExpression<'a>) {
        if let ast::Expression::Identifier(id) = &n.callee
            && matches!(id.name.as_str(), "eval" | "require" | "Function")
        {
            self.errors
                .push("runtime code loading is not authored source");
        }
        walk::walk_call_expression(self, n);
    }
    fn visit_ts_any_keyword(&mut self, _: &ast::TSAnyKeyword) {
        self.errors
            .push("owned TypeScript must not use explicit any");
    }
}
fn resolve(from: &str, import: &str) -> Result<String> {
    let raw = import.strip_suffix("?raw").unwrap_or(import);
    let path = if let Some(p) = raw.strip_prefix("@/") {
        p.to_owned()
    } else {
        format!("{}/{}", Path::new(from).parent().unwrap().display(), raw)
    };
    let mut parts = Vec::new();
    for p in path.split('/') {
        match p {
            "." | "" => {}
            ".." => {
                ensure!(parts.pop().is_some(), "module import escapes source");
            }
            _ => parts.push(p),
        }
    }
    Ok(parts.join("/"))
}
pub(super) fn load(spec_path: &Path, s: &Specification, model: &project::Project) -> Result<Files> {
    ensure!(
        s.source_modules.len() <= 128 && s.external_components.len() <= 64,
        "owned module budget exceeded"
    );
    ensure!(
        s.libraries.len() <= 1 && s.libraries.iter().all(|l| l == "three"),
        "unsupported owned library"
    );
    let base = spec_path
        .parent()
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let mut files = Files::new();
    let mut metadata = BTreeMap::new();
    let mut total = 0;
    let path_pattern = regex::Regex::new(
        r"^(?:features|components|hooks|lib|types|assets/shaders)/[a-z0-9][a-z0-9/_.-]*\.(?:ts|tsx|glsl|module\.css)$",
    )?;
    let mut portable = BTreeSet::new();
    for (path, module) in &s.source_modules {
        ensure!(
            path_pattern.is_match(path)
                && !path.starts_with("components/ui/")
                && !path.split('/').any(|p| p == ".." || p == ".")
                && !path.contains("//")
                && portable.insert(path.to_lowercase()),
            "invalid owned module path"
        );
        ensure!(
            !path.contains("compat")
                && !path.contains("recovered")
                && !module.purpose.trim().is_empty()
                && !module.evidence.is_empty(),
            "module requires authored ownership and evidence"
        );
        for source in &module.evidence {
            ensure!(
                model
                    .observations
                    .get(&source.observation)
                    .is_some_and(|o| source.node < o.capture.nodes.len()),
                "invalid owned source evidence"
            );
        }
        ensure!(
            !Path::new(&module.file).is_absolute(),
            "owned module file must be relative to specification"
        );
        let source = util::checked_path(&base, &base.join(&module.file))?;
        let bytes = super::read(&source, 256 * 1024)?;
        total += bytes.len();
        ensure!(
            total <= 4 * 1024 * 1024,
            "owned source byte budget exceeded"
        );
        let text = String::from_utf8(bytes).context("owned source must be UTF-8")?;
        ensure!(
            !text.contains("__mochi")
                && !text.contains("react.memo_cache_sentinel")
                && !text.contains("__webpack_require__")
                && !text.contains("react/compiler-runtime"),
            "recovered or compiled runtime cannot masquerade as authored source"
        );
        let mut info = Imports::default();
        if path.ends_with(".ts") || path.ends_with(".tsx") {
            let arena = oxc_allocator::Allocator::default();
            let parsed =
                oxc_parser::Parser::new(&arena, &text, oxc_span::SourceType::from_path(path)?)
                    .parse();
            ensure!(
                parsed.diagnostics.is_empty(),
                "invalid owned TypeScript: {path}"
            );
            info.visit_program(&parsed.program);
            ensure!(
                info.errors.is_empty(),
                "invalid owned source {path}: {:?}",
                info.errors
            );
        } else if path.ends_with(".module.css") {
            ensure!(
                !text.contains("@import") && !text.contains(":global"),
                "owned component CSS must stay scoped"
            );
        } else {
            ensure!(
                path.starts_with("assets/shaders/") && !text.contains("#include \""),
                "GLSL belongs in assets/shaders; use explicit raw imports"
            );
        }
        metadata.insert(path.clone(), info);
        files.insert(path.clone(), text);
    }
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (path, info) in &metadata {
        let mut dependencies = BTreeSet::new();
        for import in &info.imports {
            if import.starts_with('.') || import.starts_with("@/") {
                let resolved = resolve(path, import)?;
                let target = [
                    resolved.clone(),
                    format!("{resolved}.ts"),
                    format!("{resolved}.tsx"),
                    format!("{resolved}/index.ts"),
                ]
                .into_iter()
                .find(|p| files.contains_key(p));
                if let Some(target) = target {
                    dependencies.insert(target);
                } else {
                    ensure!(
                        matches!(
                            resolved.as_str(),
                            "lib/assets"
                                | "components/providers/motion-preference"
                                | "components/ui/button"
                                | "components/ui/dialog"
                                | "components/ui/field"
                                | "components/ui/form"
                                | "components/ui/input"
                                | "components/ui/menu"
                                | "components/ui/navigation-menu"
                                | "components/ui/switch"
                                | "components/ui/tabs"
                        ),
                        "owned import {import} in {path} is unresolved"
                    );
                }
            } else {
                ensure!(
                    matches!(
                        import.as_str(),
                        "react"
                            | "react-dom"
                            | "@tanstack/react-router"
                            | "motion"
                            | "motion/react"
                            | "motion/mini"
                    ) || import.starts_with("@base-ui/react/")
                        || (s.libraries.iter().any(|l| l == "three")
                            && (import == "three" || import.starts_with("three/addons/"))),
                    "unapproved owned dependency {import}"
                );
            }
        }
        edges.insert(path.clone(), dependencies);
    }
    let mut used = BTreeSet::new();
    fn entries(n: &Node, out: &mut BTreeSet<String>) {
        match n {
            Node::External { name, .. } => {
                out.insert(name.clone());
            }
            Node::Element { children, .. } | Node::Primitive { children, .. } => {
                for n in children {
                    entries(n, out)
                }
            }
            Node::Repeat { body, .. } => entries(body, out),
            Node::If {
                then, otherwise, ..
            } => {
                entries(then, out);
                entries(otherwise, out);
            }
            _ => {}
        }
    }
    for c in s.components.values() {
        entries(&c.body, &mut used);
    }
    ensure!(
        used.len() == s.external_components.len(),
        "unused external component entry"
    );
    let mut reached = BTreeSet::new();
    let mut pending = Vec::new();
    for name in used {
        let entry = s
            .external_components
            .get(&name)
            .context("unknown external entry")?;
        ensure!(
            metadata
                .get(&entry.module)
                .is_some_and(|m| m.exports.contains(&name)),
            "owned component {name} must be a named export of {}",
            entry.module
        );
        pending.push(entry.module.clone());
    }
    while let Some(path) = pending.pop() {
        if reached.insert(path.clone()) {
            pending.extend(edges[&path].iter().cloned());
        }
    }
    ensure!(
        reached.len() == files.len(),
        "unreachable owned modules are refused"
    );
    Ok(files)
}
