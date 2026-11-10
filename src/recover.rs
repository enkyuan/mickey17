//! Opt-in recovery of editable client modules from captured, public build output.
//! Source is parsed and lowered offline, never executed by the exporter.
mod esm;
mod native;
mod turbopack;

use crate::{cache::fingerprint, mirror, project::Project, util};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
};
use url::Url;

const INPUT_LIMIT: u64 = 128 * 1024 * 1024;
const FILE_LIMIT: u64 = 16 * 1024 * 1024;
pub(crate) fn module_references(source: &str, base: &Url) -> BTreeSet<Url> {
    native::literal_references(source, base)
}
#[derive(Default, Debug, Serialize)]
pub struct Recovery {
    pub routes: BTreeMap<String, RecoveredRoute>,
    pub sources: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub react_version: Option<String>,
    #[serde(skip)]
    pub files: BTreeMap<String, String>,
    #[serde(skip)]
    emitted_bytes: usize,
    #[serde(skip)]
    namespaces: BTreeMap<String, BTreeSet<u64>>,
}
impl Recovery {
    fn add(&mut self, path: String, source: String) -> Result<()> {
        ensure!(
            !self.files.contains_key(&path),
            "recovered source file collision"
        );
        self.emitted_bytes = self
            .emitted_bytes
            .checked_add(source.len())
            .context("recovered output size overflow")?;
        ensure!(
            self.emitted_bytes <= 128 * 1024 * 1024 && self.files.len() < 512,
            "recovered output budget exceeded"
        );
        self.files.insert(path, source);
        Ok(())
    }
}
/// A captured route that could not be recovered and is exported as a static replica.
#[derive(Debug, Serialize)]
pub struct RouteFallback {
    pub path: String,
    pub reason: String,
}
/// Recover every captured literal route that can be recovered. Routes whose
/// modules are unsupported or refused are returned as fallbacks (to be exported
/// as static replicas) instead of failing the export.
///
/// The result is always one joint `plan` of the accepted routes, exactly what an
/// explicit `--recover-client-route` selection of them produces: adapters emit
/// files whose content depends on the whole selection (Vite binding slices,
/// native element declarations and dispatchers), so separately planned routes
/// are never merged. All routes are tried together first; if that is refused,
/// each route alone decides its own refusal, the remaining routes are planned
/// together, and only if that is refused too are they added one at a time.
pub fn plan_available(
    root: &Path,
    model: &Project,
) -> Result<(Option<Recovery>, Vec<RouteFallback>)> {
    let paths: BTreeSet<String> = model.routes.values().map(|r| r.pathname.clone()).collect();
    if paths.is_empty() {
        return Ok((None, Vec::new()));
    }
    if let Ok(recovery) = plan(root, model, &paths) {
        return Ok((Some(recovery), Vec::new()));
    }
    let mut fallbacks = Vec::new();
    let mut alone = BTreeSet::new();
    let mut first = None;
    for path in paths {
        match plan(root, model, &BTreeSet::from([path.clone()])) {
            Ok(recovery) => {
                alone.insert(path);
                first.get_or_insert(recovery);
            }
            Err(error) => fallbacks.push(RouteFallback {
                path,
                reason: format!("{error:#}"),
            }),
        }
    }
    if alone.len() <= 1 {
        return Ok((first, fallbacks));
    }
    if let Ok(recovery) = plan(root, model, &alone) {
        return Ok((Some(recovery), fallbacks));
    }
    let mut accepted = BTreeSet::new();
    let mut recovery = None;
    for path in alone {
        let mut next = accepted.clone();
        next.insert(path.clone());
        match plan(root, model, &next) {
            Ok(joint) => {
                accepted = next;
                recovery = Some(joint);
            }
            Err(error) => fallbacks.push(RouteFallback {
                reason: format!(
                    "recoverable alone, but not together with {}: {error:#}",
                    if accepted.is_empty() {
                        "itself on a second read".to_owned()
                    } else {
                        accepted.iter().cloned().collect::<Vec<_>>().join(", ")
                    }
                ),
                path,
            }),
        }
    }
    fallbacks.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((recovery, fallbacks))
}
#[derive(Debug, Serialize)]
pub struct RecoveredRoute {
    pub component: String,
    pub module: u64,
    pub source: String,
    pub method: String,
}
fn read(root: &Path, path: &Path, budget: &mut u64) -> Result<String> {
    let path = util::checked_path(root, path)?;
    let file = std::fs::File::open(&path)?;
    let size = file.metadata()?.len();
    ensure!(
        size <= FILE_LIMIT && *budget + size <= INPUT_LIMIT,
        "client source input budget exceeded"
    );
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == size,
        "client input changed while reading"
    );
    *budget += size;
    Ok(String::from_utf8(bytes)?)
}
fn plain(value: &Value, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    match value {
        Value::String(s) => !s.starts_with('$'),
        Value::Array(a) => a.iter().all(|x| plain(x, depth + 1)),
        Value::Object(o) => o.iter().all(|(k, v)| {
            !matches!(k.as_str(), "__proto__" | "constructor" | "prototype") && plain(v, depth + 1)
        }),
        _ => true,
    }
}
fn candidates(
    value: &Value,
    imports: &BTreeMap<String, (u64, String)>,
    out: &mut Vec<(u64, String, Value)>,
    depth: usize,
) {
    if depth > 64 {
        return;
    }
    if let Some(a) = value.as_array() {
        if a.len() == 4
            && a[0] == "$"
            && let Some(id) = a[1].as_str().and_then(|s| s.strip_prefix("$L"))
            && let Some((module, export)) = imports.get(id)
            && a[3].is_object()
            && plain(&a[3], 0)
        {
            out.push((*module, export.clone(), a[3].clone()));
        }
        for child in a {
            candidates(child, imports, out, depth + 1)
        }
    } else if let Some(o) = value.as_object() {
        for child in o.values() {
            candidates(child, imports, out, depth + 1)
        }
    }
}
/// Parse only newline-framed JSON Flight records. Binary/text/server-reference
/// rows are not interpreted as component props. Unknown props fail closed.
fn flight(source: &str) -> Result<Vec<(u64, String, Value)>> {
    let mut imports = BTreeMap::new();
    let mut rows = Vec::new();
    let records = std::cell::RefCell::new(Vec::new());
    crate::flight::map_rows_with_text(
        source,
        &|row| {
            records.borrow_mut().push(row.to_owned());
            String::new()
        },
        &|_| String::new(),
    )?;
    for line in records.into_inner() {
        let Some((id, body)) = line.split_once(':') else {
            continue;
        };
        if id.is_empty() || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        if let Some(body) = body.strip_prefix('I') {
            let v: Value = serde_json::from_str(body)?;
            if let (Some(module), Some(export)) = (v[0].as_u64(), v[2].as_str()) {
                imports.insert(id.to_owned(), (module, export.into()));
            }
        } else if let Ok(row) = serde_json::from_str::<Value>(body) {
            rows.push(row)
        }
    }
    let mut out = Vec::new();
    for row in rows {
        candidates(&row, &imports, &mut out, 0)
    }
    Ok(out)
}
fn emit(
    id: u64,
    registry: &turbopack::Registry,
    recovery: &mut Recovery,
    visiting: &mut BTreeSet<u64>,
) -> Result<String> {
    let module = registry
        .modules
        .get(&id)
        .with_context(|| format!("missing client dependency {id}"))?;
    let hash = fingerprint(module.factory.as_bytes());
    let identity = fingerprint(&serde_json::to_vec(&(&hash, &module.ids))?);
    let name = format!("Module{}", &identity[..16]);
    let path = format!("src/compat/recovered/{name}.js");
    if recovery.files.contains_key(&path) {
        ensure!(
            recovery
                .namespaces
                .get(&path)
                .is_some_and(|ids| ids.contains(&id)),
            "client namespace {id} is not exported"
        );
        return Ok(name);
    }
    ensure!(
        visiting.len() < 128 && visiting.insert(id),
        "cyclic or excessive client dependency graph at {id}"
    );
    let mut exported = BTreeSet::new();
    let body = if let Some(bridge) = registry.bridge(id) {
        exported.extend(module.ids.iter().copied());
        format!(
            "import * as bridge from {};\n{}",
            serde_json::to_string(bridge)?,
            module
                .ids
                .iter()
                .map(|id| format!("export const Module{id}=bridge;\n"))
                .collect::<String>()
        )
    } else {
        let lowered = turbopack::lower(&module.factory, &module.ids)
            .with_context(|| format!("lower client module {id} from {}", module.source))?;
        ensure!(
            lowered.exports.contains_key(&id),
            "client namespace {id} is not exported"
        );
        exported.extend(lowered.exports.keys().copied());
        let mut source = exported
            .iter()
            .map(|id| format!("const __mochiModule{id}={{}};\n"))
            .collect::<String>();
        for dependency in lowered.imports {
            let target = emit(dependency, registry, recovery, visiting)?;
            source.push_str(&format!(
                "import {{Module{dependency} as __mochiImport{dependency}}} from './{target}.js';\n"
            ));
        }
        source.push_str(&lowered.body);
        for namespace in &exported {
            source.push_str(&format!(
                "\nexport {{__mochiModule{namespace} as Module{namespace}}};\n"
            ));
        }
        source
    };
    visiting.remove(&id);
    let provenance = format!(
        "// Recovered public client source, statically lowered and formatted.\n// Input: {} (factory sha256: {hash}). Module {id}.\n",
        serde_json::to_string(&module.source)?
    );
    recovery.namespaces.insert(path.clone(), exported.clone());
    recovery.add(path, provenance + &turbopack::format(&body)?)?;
    recovery.add(
        format!("src/compat/recovered/{name}.d.ts"),
        exported
            .iter()
            .map(|id| format!("export declare const Module{id}: Record<string,unknown>;\n"))
            .collect(),
    )?;
    ensure!(
        recovery.files.len() <= 512,
        "recovered module output count exceeded"
    );
    Ok(name)
}
/// Recover only unambiguous, data-prop client roots with observable DOM behavior.
/// Every selected route must lower completely; there is no opaque bundle fallback.
pub fn plan(root: &Path, model: &Project, paths: &BTreeSet<String>) -> Result<Recovery> {
    ensure!(
        !paths.is_empty(),
        "select at least one literal client route"
    );
    let has_flight = paths.iter().any(|path| {
        model
            .routes
            .values()
            .filter(|route| &route.pathname == path)
            .any(|route| {
                Url::parse(&route.url)
                    .ok()
                    .and_then(|url| mirror::output_path(&root.join("site"), &url, true).ok())
                    .is_some_and(|html| html.with_file_name("index.rsc").is_file())
            })
    });
    if !has_flight {
        if native::candidate(root, model, paths)? {
            return native::plan(root, model, paths);
        }
        return esm::plan(root, model, paths);
    }
    let mut budget = 0;
    let mut recovery = Recovery::default();
    let site = root.join("site");
    for path in paths {
        let routes = model
            .routes
            .values()
            .filter(|r| &r.pathname == path)
            .collect::<Vec<_>>();
        ensure!(
            routes.len() == 1,
            "client recovery requires one literal route state for {path}"
        );
        let route = routes[0];
        let url = Url::parse(&route.url)?;
        let html = mirror::output_path(&site, &url, true)?;
        let mut registry = turbopack::Registry::default();
        let mut scripts = BTreeSet::new();
        for observation in &route.observations {
            for resource in &model.observations[observation].capture.resources {
                // Module preloads arrive with initiator `link`, not `script`.
                // Keep the observed chunk inventory rather than inferring execution.
                let script = Url::parse(&resource.url)?;
                if script.origin() != url.origin()
                    || !script.path().starts_with("/_next/static/chunks/")
                    || !script.path().ends_with(".js")
                    || script.query().is_some()
                    || !scripts.insert(script.to_string())
                {
                    continue;
                }
                let file = mirror::output_path(&site, &script, false)?;
                let source = read(root, &file, &mut budget)?;
                let relative = file.strip_prefix(root)?.to_string_lossy().into_owned();
                registry.add(&relative, &source)?;
                recovery
                    .sources
                    .insert(relative, fingerprint(source.as_bytes()));
            }
        }
        let rsc = html.with_file_name("index.rsc");
        let payload = read(root, &rsc, &mut budget)
            .with_context(|| format!("client recovery needs saved Flight records for {path}"))?;
        recovery.sources.insert(
            rsc.strip_prefix(root)?.to_string_lossy().into_owned(),
            fingerprint(payload.as_bytes()),
        );
        let mut roots = flight(&payload)?
            .into_iter()
            .filter(|(id, export, _)| {
                export == "default"
                    && registry.modules.get(id).is_some_and(|m| {
                        m.factory.contains("querySelector(")
                            && m.factory.contains("useEffect")
                            && m.factory.contains("jsx")
                    })
            })
            .collect::<Vec<_>>();
        roots.sort_by_key(|(id, _, props)| (*id, props.to_string()));
        roots.dedup();
        ensure!(
            roots.len() == 1,
            "expected one unambiguous behavioral client root for {path}, found {}",
            roots.len()
        );
        let (id, _, props) = roots.remove(0);
        let lowered = turbopack::lower(&registry.modules[&id].factory, &registry.modules[&id].ids)?;
        ensure!(
            lowered
                .exports
                .get(&id)
                .is_some_and(|exports| exports.contains("default")),
            "client root has no default export"
        );
        let module = emit(id, &registry, &mut recovery, &mut BTreeSet::new())?;
        let component = format!("Client{}", &fingerprint(route.url.as_bytes())[..16]);
        let props = serde_json::to_string_pretty(&props)?;
        recovery.add(format!("src/components/{component}.tsx"),format!("import type {{ComponentType}} from 'react'\nimport {{Module{id} as recovered}} from '../compat/recovered/{module}.js'\nconst Page=recovered.default as ComponentType<Record<string,unknown>>\n// Plain captured client props. Original public source, not generated backend logic.\nconst props={props}\nexport function {component}(){{return <Page {{...props}} />}}\n"))?;
        recovery.routes.insert(
            path.clone(),
            RecoveredRoute {
                component,
                module: id,
                source: registry.modules[&id].source.clone(),
                method: "static_turbopack_to_editable_esm".into(),
            },
        );
    }
    Ok(recovery)
}
pub fn verify(root: &Path, recovery: &Recovery) -> Result<()> {
    let mut budget = 0;
    for (relative, hash) in &recovery.sources {
        let source = read(root, &root.join(relative), &mut budget)?;
        ensure!(
            fingerprint(source.as_bytes()) == *hash,
            "client source changed before publication"
        );
    }
    Ok(())
}
pub fn localize(
    recovery: &mut Recovery,
    base: &Url,
    assets: &BTreeMap<String, String>,
) -> Result<()> {
    let mut bytes = 0usize;
    for (path, source) in &mut recovery.files {
        if path.ends_with(".d.ts") {
            continue;
        }
        let source_type = oxc_span::SourceType::from_path(path)?;
        *source = turbopack::localize(source, source_type, base, assets)?;
        bytes = bytes
            .checked_add(source.len())
            .context("localized source size overflow")?;
        ensure!(
            bytes <= 128 * 1024 * 1024,
            "localized client output budget exceeded"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovered_output_budget_is_checked_before_insertion() {
        let mut recovery = Recovery {
            emitted_bytes: 128 * 1024 * 1024 - 1,
            ..Recovery::default()
        };
        assert!(
            recovery
                .add("src/compat/recovered/large.js".into(), "xx".into())
                .is_err()
        );
        assert!(recovery.files.is_empty());
    }
    #[test]
    fn flight_selects_data_props_and_rejects_executable_reference_encodings() {
        let rows = "a:I[123,[],\"default\"]\nb:[\"$\",\"$La\",null,{\"title\":\"hello\",\"items\":[1,2]}]\nc:[\"$\",\"$La\",null,{\"action\":\"$F1\"}]\nd:[\"$\",\"$La\",null,{\"child\":\"$L1\"}]\ne:[\"$\",\"$La\",null,{\"__proto__\":{}}]\n";
        let candidates = flight(rows).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, 123);
        assert_eq!(candidates[0].2["title"], "hello");
        assert!(!plain(
            &serde_json::json!({"nested":{"constructor":"x"}}),
            0
        ));
        let fake = "\nb:[\"$\",\"$La\",null,{\"title\":\"not a real component record\"}]\n";
        let framed = format!("a:I[123,[],\"default\"]\n0:T{:x},{fake}", fake.len());
        assert!(
            flight(&framed).unwrap().is_empty(),
            "length-framed text is not Flight syntax"
        );
    }
}
