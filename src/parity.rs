//! Offline comparison of independently captured browser states. A match is scoped
//! to a declared contract, never a claim about unobserved routes or interactions.
use crate::cache::fingerprint;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const MAX_BYTES: u64 = 32 * 1024 * 1024;
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub schema_version: u32,
    pub reference_sha256: String,
    pub project_sha256: String,
    /// Every route must have desktop/mobile initial, bottom and reduced-motion states.
    pub routes: BTreeSet<String>,
    pub states: BTreeMap<String, StateContract>,
    /// Explicit unresolved coverage prevents qualification.
    pub gaps: Vec<String>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateContract {
    pub route: String,
    pub viewport: [u32; 2],
    pub reduced_motion: bool,
    pub phase: Phase,
    /// Reference-derived semantic anchors. Include every section, control and graphic.
    pub inventory: BTreeSet<String>,
    /// Numeric tolerances are in CSS pixels and cannot exceed one pixel.
    pub tolerances: BTreeMap<String, f64>,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Initial,
    Scroll,
    Interaction,
    Bottom,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub schema_version: u32,
    pub project_sha256: Option<String>,
    pub states: BTreeMap<String, State>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub route: String,
    pub viewport: [u32; 2],
    pub reduced_motion: bool,
    pub inventory: BTreeSet<String>,
    /// Flattened measurements, e.g. hero.bounds.height or navigation.expanded.
    pub facts: BTreeMap<String, Value>,
    pub errors: Vec<String>,
    pub settled: bool,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub status: String,
    pub contract_sha256: String,
    pub reference_sha256: String,
    pub candidate_sha256: String,
    pub project_sha256: String,
    pub states_checked: usize,
    pub facts_checked: usize,
    pub failures: Vec<String>,
    pub scope: &'static str,
}
fn read(path: &Path) -> Result<Vec<u8>> {
    crate::authored::read(path, MAX_BYTES)
}
/// Hash every editable runtime input, including newly added files. Build artifacts
/// and provenance are excluded. Symlinks are refused, not followed outside the app.
pub fn project_fingerprint(root: &Path) -> Result<String> {
    let root = root.canonicalize()?;
    ensure!(
        root.join("src/routes").is_dir(),
        "expected generated src/routes"
    );
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    for entry in walkdir::WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                ".git"
                    | ".mochi"
                    | ".output"
                    | ".tanstack"
                    | ".cache"
                    | ".DS_Store"
                    | "node_modules"
                    | "dist"
                    | "target"
            )
        })
    {
        let entry = entry?;
        ensure!(
            !entry.file_type().is_symlink(),
            "symlink in project inputs: {}",
            entry.path().display()
        );
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        // Source, public assets, package manifest/lock and top-level runtime configuration.
        if !(relative.starts_with("src/")
            || relative.starts_with("public/")
            || entry.depth() == 1 && !relative.ends_with(".md"))
        {
            continue;
        }
        total += entry.metadata()?.len();
        ensure!(
            files.len() < 20_000 && total <= 512 * 1024 * 1024,
            "project fingerprint budget exceeded"
        );
        files.insert(relative, fingerprint(&read(entry.path())?));
    }
    ensure!(files.contains_key("package.json"), "missing package.json");
    Ok(fingerprint(&serde_json::to_vec(&files)?))
}
fn validate(contract: &Contract, reference: &Capture, candidate: &Capture) -> Result<()> {
    ensure!(
        contract.schema_version == 1
            && reference.schema_version == 1
            && candidate.schema_version == 1,
        "unsupported parity schema"
    );
    ensure!(
        !contract.routes.is_empty() && !contract.states.is_empty() && contract.states.len() <= 4096,
        "empty or oversized parity coverage"
    );
    ensure!(
        contract.gaps.is_empty(),
        "unresolved parity coverage: {}",
        contract.gaps.join("; ")
    );
    for route in &contract.routes {
        ensure!(
            route.starts_with('/') && !route.starts_with("//") && !route.contains(['?', '#']),
            "invalid route"
        );
        for mobile in [true, false] {
            for phase in [Phase::Initial, Phase::Scroll, Phase::Bottom] {
                ensure!(
                    contract.states.values().any(|s| &s.route == route
                        && (s.viewport[0] < 768) == mobile
                        && !s.reduced_motion
                        && s.phase == phase),
                    "missing initial/scroll/bottom viewport coverage for {route}"
                );
            }
            ensure!(
                contract.states.values().any(|s| &s.route == route
                    && (s.viewport[0] < 768) == mobile
                    && s.reduced_motion),
                "missing reduced motion coverage for {route}"
            );
        }
    }
    for (key, state) in &contract.states {
        ensure!(
            !key.is_empty() && contract.routes.contains(&state.route),
            "unknown route or empty state"
        );
        ensure!(
            state.viewport.iter().all(|v| *v >= 100 && *v <= 8192),
            "invalid viewport"
        );
        ensure!(
            !state.inventory.is_empty(),
            "empty section/control/graphic inventory: {key}"
        );
        ensure!(
            state.tolerances.iter().all(|(key, v)| v.is_finite()
                && (0.0..=1.0).contains(v)
                && key
                    .rsplit_once(".bounds.")
                    .is_some_and(|(_, axis)| matches!(
                        axis,
                        "x" | "y" | "width" | "height" | "top" | "left" | "right" | "bottom"
                    ))),
            "invalid tolerance: {key}"
        );
    }
    Ok(())
}
pub fn compare(
    contract: &Contract,
    reference: &Capture,
    candidate: &Capture,
) -> Result<(usize, Vec<String>)> {
    validate(contract, reference, candidate)?;
    let mut failures = Vec::new();
    let expected: BTreeSet<_> = contract.states.keys().collect();
    for (name, capture) in [("reference", reference), ("candidate", candidate)] {
        if capture.states.keys().collect::<BTreeSet<_>>() != expected {
            failures.push(format!("{name}: state coverage differs from contract"));
        }
    }
    let mut count = 0;
    for (key, requirement) in &contract.states {
        let (Some(a), Some(b)) = (reference.states.get(key), candidate.states.get(key)) else {
            continue;
        };
        for (label, state) in [("reference", a), ("candidate", b)] {
            if state.route != requirement.route
                || state.viewport != requirement.viewport
                || state.reduced_motion != requirement.reduced_motion
            {
                failures.push(format!("{key}: {label} context mismatch"));
            }
            if !state.settled || !state.errors.is_empty() {
                failures.push(format!("{key}: {label} unsettled or browser errors"));
            }
            if state.inventory != requirement.inventory {
                failures.push(format!(
                    "{key}: {label} missing or extra sections, controls, or graphics"
                ));
            }
            if state.facts.is_empty() {
                failures.push(format!("{key}: {label} empty measurements"));
            }
            for anchor in &requirement.inventory {
                if !state
                    .facts
                    .keys()
                    .any(|k| k.starts_with(&format!("{anchor}.")))
                {
                    failures.push(format!("{key}: {label} unmeasured anchor {anchor}"));
                }
            }
        }
        for name in requirement.tolerances.keys() {
            if !a.facts.get(name).is_some_and(Value::is_number) {
                failures.push(format!("{key}: tolerance has no numeric reference {name}"));
            }
        }
        if a.facts.keys().collect::<BTreeSet<_>>() != b.facts.keys().collect::<BTreeSet<_>>() {
            failures.push(format!("{key}: measurement coverage differs"));
        }
        for (name, value) in &a.facts {
            count += 1;
            let Some(other) = b.facts.get(name) else {
                continue;
            };
            let matches = match (value.as_f64(), other.as_f64()) {
                (Some(x), Some(y)) => {
                    (x - y).abs() <= *requirement.tolerances.get(name).unwrap_or(&0.0)
                }
                _ => value == other && !value.is_null() && !value.is_object() && !value.is_array(),
            };
            if !matches {
                failures.push(format!("{key}: {name}: expected {value}, got {other}"));
            }
        }
    }
    Ok((count, failures))
}

#[derive(Default)]
struct RouteLiterals {
    paths: Vec<String>,
    non_literal: bool,
}
impl<'a> oxc_ast_visit::Visit<'a> for RouteLiterals {
    fn visit_call_expression(&mut self, call: &oxc_ast::ast::CallExpression<'a>) {
        if let oxc_ast::ast::Expression::Identifier(name) = &call.callee
            && name.name.as_str() == "createFileRoute"
        {
            if let Some(oxc_ast::ast::Argument::StringLiteral(path)) = call.arguments.first()
                && let Some(value) = path.value.as_str()
            {
                self.paths.push(value.to_owned());
            } else {
                self.non_literal = true;
            }
        }
        oxc_ast_visit::walk::walk_call_expression(self, call);
    }
}
pub fn check(
    project: &Path,
    contract_path: &Path,
    reference_path: &Path,
    candidate_path: &Path,
    out: &Path,
) -> Result<()> {
    // Each invocation has a distinct artifact. A failed input validation cannot
    // leave a previous successful report masquerading as this invocation.
    ensure!(
        !out.exists(),
        "parity report already exists; choose a new output path"
    );
    let contract_bytes = read(contract_path)?;
    let reference_bytes = read(reference_path)?;
    let candidate_bytes = read(candidate_path)?;
    let contract: Contract =
        serde_json::from_slice(&contract_bytes).context("parse parity contract")?;
    let reference: Capture =
        serde_json::from_slice(&reference_bytes).context("parse reference states")?;
    let candidate: Capture =
        serde_json::from_slice(&candidate_bytes).context("parse candidate states")?;
    let project_sha256 = project_fingerprint(project)?;
    let mut routes = BTreeSet::new();
    for entry in walkdir::WalkDir::new(project.join("src/routes")).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file()
            || entry
                .path()
                .extension()
                .is_none_or(|x| !matches!(x.to_str(), Some("ts" | "tsx" | "js" | "jsx")))
            || entry
                .path()
                .file_stem()
                .is_some_and(|name| name == "__root")
        {
            continue;
        }
        let source = String::from_utf8(read(entry.path())?)?;
        let arena = oxc_allocator::Allocator::default();
        let parsed = oxc_parser::Parser::new(
            &arena,
            &source,
            oxc_span::SourceType::from_path(entry.path())?,
        )
        .parse();
        ensure!(parsed.diagnostics.is_empty(), "invalid route source");
        use oxc_ast_visit::Visit;
        let mut declarations = RouteLiterals::default();
        declarations.visit_program(&parsed.program);
        ensure!(
            !declarations.non_literal && declarations.paths.len() == 1,
            "route must have exactly one inspectable literal createFileRoute; parity coverage requires review"
        );
        ensure!(
            routes.insert(declarations.paths.remove(0)),
            "duplicate route declaration"
        );
    }
    ensure!(
        routes
            .iter()
            .map(|route| route.trim_end_matches('/'))
            .collect::<BTreeSet<_>>()
            == contract
                .routes
                .iter()
                .map(|route| route.trim_end_matches('/'))
                .collect::<BTreeSet<_>>(),
        "contract does not cover every current generated route"
    );
    ensure!(
        contract.reference_sha256 == fingerprint(&reference_bytes),
        "reference capture changed"
    );
    ensure!(
        contract.project_sha256 == project_sha256
            && candidate.project_sha256.as_ref() == Some(&project_sha256),
        "stale candidate: project source changed"
    );
    let (facts_checked, failures) = compare(&contract, &reference, &candidate)?;
    ensure!(
        project_fingerprint(project)? == project_sha256,
        "project changed during parity check"
    );
    let passed = failures.is_empty();
    let report = Report {
        status: if passed {
            "matched_declared_contract"
        } else {
            "failed"
        }
        .into(),
        contract_sha256: fingerprint(&contract_bytes),
        reference_sha256: fingerprint(&reference_bytes),
        candidate_sha256: fingerprint(&candidate_bytes),
        project_sha256,
        states_checked: contract.states.len(),
        facts_checked,
        failures,
        scope: "Only recorded states and measured facts. Capture provenance and coverage require independent review; this is not proof for unobserved behavior or other websites.",
    };
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)?;
    file.write_all(&serde_json::to_vec_pretty(&report)?)?;
    ensure!(
        passed,
        "parity failed: {} differences; see {}",
        report.failures.len(),
        out.display()
    );
    Ok(())
}
