use mochi::{export, project, recover};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};
#[path = "support/source_fixture.rs"]
mod source_fixture;

fn fixture(root: &Path) {
    source_fixture::fixture(root, 1, 2);
    std::fs::create_dir_all(root.join("site/catalog")).unwrap();
    std::fs::create_dir_all(root.join("site/_next/static/chunks")).unwrap();
    std::fs::write(root.join("site/_next/static/chunks/client.js"),r#"globalThis.TURBOPACK.push([document.currentScript,11,e=>{const useEffect=()=>{};const jsx=(type,props)=>({$$typeof:Symbol.for('react.transitional.element'),type,key:null,props});function Root(props){useEffect(()=>document.querySelector('svg'));return jsx('svg',{title:props.title})}throw new Error('DO_NOT_EXECUTE_CAPTURED_SOURCE');e.s(['default',()=>Root]);}]);"#).unwrap();
    std::fs::write(
        root.join("site/catalog/index.rsc"),
        "1:I[11,[],\"default\"]\n0:[\"$\",\"$L1\",null,{\"title\":\"Editable vector page\"}]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("site/source.css"),
        "@keyframes reveal{to{opacity:1}} .intro{opacity:0;animation:var(--intro-animation)}",
    )
    .unwrap();
    for view in 0..2 {
        let p = root.join(format!("reconstruction/pages/0-{view}.json"));
        let mut capture: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        capture["resources"] = json!([{"url":"https://example.test/_next/static/chunks/client.js","initiator":"script","response_status":200},{"url":"https://example.test/brand.svg","initiator":"img","response_status":200}]);
        capture["stylesheets"][0]["href"] = json!("https://example.test/source.css");
        std::fs::write(p, serde_json::to_vec(&capture).unwrap()).unwrap();
    }
}
#[test]
fn recovery_is_offline_auditable_and_detects_changed_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    fixture(&root);
    let model = project::build(&root).unwrap();
    let paths = BTreeSet::from(["/catalog".into()]);
    let recovery = recover::plan(&root, &model, &paths).unwrap();
    assert_eq!(recovery.routes["/catalog"].module, 11);
    assert!(
        recovery
            .files
            .keys()
            .all(|p| p.starts_with("src/components/") || p.starts_with("src/compat/recovered/"))
    );
    assert!(
        recovery
            .files
            .values()
            .any(|s| s.contains("DO_NOT_EXECUTE_CAPTURED_SOURCE"))
    );
    recover::verify(&root, &recovery).unwrap();
    std::fs::write(root.join("site/_next/static/chunks/client.js"), "changed").unwrap();
    assert!(recover::verify(&root, &recovery).is_err());
}
#[test]
fn selected_route_export_uses_client_source_and_refuses_unsupported_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    fixture(&root);
    project::build(&root).unwrap();
    let paths = BTreeSet::from(["/catalog".into()]);
    let out = temp.path().join("project");
    let report = export::tanstack_start_with_clients(&root, &out, false, &paths).unwrap();
    assert_eq!(report.client_recovery.unwrap().routes.len(), 1);
    assert!(
        std::fs::read_to_string(out.join("src/features/pages/catalog/page.tsx"))
            .unwrap()
            .contains("<Client")
    );
    assert_eq!(report.verification, "not_run");
    assert!(
        std::fs::read_dir(out.join("src/styles/compat"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| std::fs::read_to_string(entry.path())
                .unwrap()
                .contains("animation:var(--intro-animation)")),
        "recovery preserves original variable animation shorthand rather than lossy CSSOM"
    );
    assert_eq!(
        std::fs::read(out.join("public/brand.svg")).unwrap(),
        b"<svg/>",
        "dynamic passive asset paths remain available"
    );
    let next = temp.path().join("refused");
    assert!(
        export::tanstack_start_with_clients(
            &root,
            &next,
            false,
            &BTreeSet::from(["/unknown".into()])
        )
        .is_err()
    );
    assert!(!next.exists());
}

#[test]
fn native_recovery_emits_pristine_markup_and_parsed_modules_without_running_them() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    source_fixture::fixture(&root, 1, 1);
    std::fs::create_dir(root.join("site/catalog")).unwrap();
    std::fs::write(root.join("site/catalog/index.html"),r#"<!doctype html><html><body><external-infra>NONVISUAL_SCAFFOLD</external-infra><live-counter title="A &amp; B"><h1>Original text</h1><img src="/brand.svg"></live-counter><script type="module" src="/entry.js"></script><script>throw Error('INLINE_MUST_BE_OMITTED')</script></body></html>"#).unwrap();
    std::fs::write(root.join("site/entry.js"),r#"import {paint} from './paint.js';class Counter extends HTMLElement{connectedCallback(){paint(this)}};customElements.define('live-counter',Counter);throw Error('OFFLINE_EXPORT_MUST_NOT_EXECUTE');"#).unwrap();
    std::fs::write(
        root.join("site/paint.js"),
        "export function paint(node){node.textContent='Live counter';}",
    )
    .unwrap();
    for relative in [
        "reconstruction/pages/0-0.json",
        "reconstruction/manifests/0.json",
    ] {
        let file = root.join(relative);
        let mut value: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        value["url"] = json!("https://example.test/catalog/?kind=0&sort=asc");
        if let Some(nodes) = value["nodes"].as_array_mut() {
            for tag in ["external-infra", "live-counter"] {
                let mut node = source_fixture::element(Some(1), tag, &[]);
                node["bounds"]["width"] = json!(0);
                node["bounds"]["height"] = json!(0);
                nodes.push(node);
            }
        }
        std::fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    let model = project::build(&root).unwrap();
    let paths = BTreeSet::from(["/catalog/".into()]);
    let recovery = recover::plan(&root, &model, &paths).unwrap();
    assert_eq!(
        recovery.routes["/catalog/"].method,
        "native_custom_elements_esm"
    );
    assert_eq!(recovery.sources.len(), 3);
    recover::verify(&root, &recovery).unwrap();
    let component = recovery
        .files
        .iter()
        .find(|(name, _)| name.starts_with("src/components/"))
        .unwrap()
        .1;
    assert!(component.contains("<live-counter"));
    assert!(component.contains("Original text"));
    assert!(!component.contains("NONVISUAL_SCAFFOLD"));
    assert!(recovery.notes.iter().any(|n| n.contains("external-infra")));
    assert!(!component.contains("INLINE_MUST_BE_OMITTED"));
    assert!(component.contains("A & B"));
    let modules = recovery
        .files
        .iter()
        .filter(|(name, _)| name.ends_with(".js"))
        .map(|(_, s)| s.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(modules.contains("customElements.define"));
    assert!(modules.contains("OFFLINE_EXPORT_MUST_NOT_EXECUTE"));
    assert!(!modules.contains("./paint.js"));
    let destination = temp.path().join("project");
    export::tanstack_start_with_clients(&root, &destination, false, &paths).unwrap();
    assert!(destination.join("src/recovered-elements.d.ts").exists());
    std::fs::write(root.join("site/paint.js"), "changed").unwrap();
    assert!(recover::verify(&root, &recovery).is_err());
}

#[test]
fn native_recovery_refuses_dynamic_import_targets_instead_of_replaying_them() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    source_fixture::fixture(&root, 1, 1);
    std::fs::create_dir(root.join("site/catalog")).unwrap();
    std::fs::write(
        root.join("site/catalog/index.html"),
        "<native-example></native-example><script type=module src=/entry.js></script>",
    )
    .unwrap();
    std::fs::write(root.join("site/entry.js"),"customElements.define('native-example',class extends HTMLElement{});const path=location.hash;import(path);").unwrap();
    let model = project::build(&root).unwrap();
    let result = recover::plan(&root, &model, &BTreeSet::from(["/catalog".into()]));
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("nonliteral native module import")
    );
}

#[test]
fn native_markup_import_targets_require_exact_query_response_receipts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    source_fixture::fixture(&root, 1, 1);
    std::fs::create_dir(root.join("site/catalog")).unwrap();
    std::fs::create_dir(root.join("site/.mochi-source")).unwrap();
    std::fs::write(root.join("site/catalog/index.html"),"<native-widget data-module='/widget.js?v=1'></native-widget><script type=module src=/entry.js></script>").unwrap();
    std::fs::write(root.join("site/entry.js"),"customElements.define('native-widget',class extends HTMLElement{connectedCallback(){import(this.getAttribute('data-module'))}})").unwrap();
    std::fs::write(
        root.join("site/widget.js"),
        "export const value='WRONG_PATH_ALIAS';",
    )
    .unwrap();
    let url = "https://example.test/widget.js?v=1";
    let receipt = root.join("site/.mochi-source").join(format!(
        "{}.blob",
        mochi::cache::fingerprint(url.as_bytes())
    ));
    std::fs::write(&receipt, "export const value='EXACT_VERSION_RESPONSE';").unwrap();
    let model = project::build(&root).unwrap();
    let paths = BTreeSet::from(["/catalog".into()]);
    let recovery = recover::plan(&root, &model, &paths).unwrap();
    let sources = recovery
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(sources.contains("EXACT_VERSION_RESPONSE"));
    assert!(!sources.contains("WRONG_PATH_ALIAS"));
    assert!(sources.contains("__mochiImportRecorded"));
    std::fs::remove_file(receipt).unwrap();
    assert!(
        recover::plan(&root, &model, &paths)
            .unwrap_err()
            .to_string()
            .contains("exact full-URL response receipt")
    );
}

fn files(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .map(|e| e.unwrap())
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            (
                e.path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}
/// `/catalog` has a recoverable client program; `/other` has none.
fn mixed_capture(root: &Path) {
    source_fixture::fixture(root, 1, 2);
    fixture(root);
    // Second literal route without any client program, copied from the first.
    for view in 0..2 {
        let text = std::fs::read(root.join(format!("reconstruction/pages/0-{view}.json"))).unwrap();
        let mut value: Value = serde_json::from_slice(&text).unwrap();
        value["url"] = json!("https://example.test/other");
        value["resources"] = json!([]);
        std::fs::write(
            root.join(format!("reconstruction/pages/1-{view}.json")),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }
    let mut manifest: Value = serde_json::from_slice(
        &std::fs::read(root.join("reconstruction/manifests/0.json")).unwrap(),
    )
    .unwrap();
    manifest["url"] = json!("https://example.test/other");
    manifest["snapshots"] = json!([
        "reconstruction/pages/1-0.json",
        "reconstruction/pages/1-1.json"
    ]);
    std::fs::write(
        root.join("reconstruction/manifests/1.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    // The first route is the recoverable one: make its URL a plain literal path.
    for relative in [
        "reconstruction/pages/0-0.json",
        "reconstruction/pages/0-1.json",
        "reconstruction/manifests/0.json",
    ] {
        let file = root.join(relative);
        let mut value: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        value["url"] = json!("https://example.test/catalog");
        std::fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
    }
}
#[test]
fn default_recovery_covers_every_recoverable_route_and_reports_the_fallbacks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    mixed_capture(&root);
    project::build(&root).unwrap();
    let before = files(&root);
    let out = temp.path().join("project");
    let report = export::tanstack_start_recovering(&root, &out, false).unwrap();
    assert_eq!(before, files(&root), "export must not alter the capture");
    assert_eq!(
        report.client_recovery_mode,
        "default_every_route_with_per_route_fallback"
    );
    let mode = |path: &str| report.route_modes.iter().find(|m| m.path == path).unwrap();
    assert_eq!(mode("/catalog").mode, "recovered_client_program");
    assert!(
        mode("/catalog")
            .component
            .as_deref()
            .unwrap()
            .starts_with("Client")
    );
    assert_eq!(mode("/other").mode, "static_structural_replica");
    let reason = mode("/other").reason.as_deref().unwrap();
    assert!(
        reason.starts_with("fallback: client recovery refused for this route (")
            && reason.contains("client"),
        "{reason}"
    );
    assert_eq!(
        report.runtime,
        "recovered_client_programs_on_some_routes_static_replica_on_others"
    );
    // The recovered route mounts recovered code; the other keeps its static markup.
    let page = |name: &str| std::fs::read_to_string(out.join(name)).unwrap();
    assert!(page("src/features/pages/catalog/page.tsx").contains("<Client"));
    assert!(!page("src/features/pages/other/page.tsx").contains("<Client"));
    // Every recovered file is listed with the digest of the bytes written.
    assert!(!report.recovered_files.is_empty());
    for (path, digest) in &report.recovered_files {
        assert_eq!(
            &mochi::cache::fingerprint(&std::fs::read(out.join(path)).unwrap()),
            digest,
            "{path}"
        );
    }
    let json: Value =
        serde_json::from_slice(&std::fs::read(out.join(".mochi/export.json")).unwrap()).unwrap();
    assert_eq!(json["route_modes"].as_array().unwrap().len(), 2);
    assert!(
        json["client_recovery"]["sources"]
            .as_object()
            .unwrap()
            .len()
            >= 2
    );
    // Docs say which route runs what, and do not claim finished conversion or verified behavior.
    for doc in ["README.md", "EDITING.md"] {
        let text = page(doc);
        assert!(
            text.contains("1 of 2 captured route(s) run recovered original client code"),
            "{text}"
        );
        assert!(text.contains("| `/catalog` | recovered client program"));
        assert!(text.contains("| `/other` | static structural replica | fallback:"));
        assert!(text.contains("not semantic source") && text.contains("unverified"));
    }
    assert!(page("EDITING.md").contains("conversion complete while a route still depends"));
    let limitations = report.limitations.join("\n");
    assert!(limitations.contains("Routes /catalog run recovered"));
    assert!(limitations.contains("Routes /other are static structural replicas"));
    assert!(limitations.contains("On static routes: Animation is not replayed"));
    // Two exports of one capture are byte-identical.
    let again = temp.path().join("again");
    export::tanstack_start_recovering(&root, &again, false).unwrap();
    assert_eq!(files(&out), files(&again));
    // Recovered inputs are covered by the stale-input check.
    let (recovery, fallbacks) =
        recover::plan_available(&root, &project::build(&root).unwrap()).unwrap();
    assert_eq!(fallbacks.len(), 1);
    let recovery = recovery.unwrap();
    assert!(recovery.sources.keys().any(|p| p.ends_with("client.js")));
    recover::verify(&root, &recovery).unwrap();
    std::fs::write(root.join("site/_next/static/chunks/client.js"), "changed").unwrap();
    assert!(recover::verify(&root, &recovery).is_err());
}
#[test]
fn explicit_selection_stays_strict_and_static_mode_recovers_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    mixed_capture(&root);
    project::build(&root).unwrap();
    // A selected route that cannot be recovered fails the whole export, with nothing published.
    let strict = temp.path().join("strict");
    assert!(
        export::tanstack_start_with_clients(
            &root,
            &strict,
            false,
            &BTreeSet::from(["/other".into()])
        )
        .is_err()
    );
    assert!(!strict.exists());
    // Library `tanstack_start` and CLI --no-recover-client are static everywhere.
    let static_out = temp.path().join("static");
    let report = export::tanstack_start(&root, &static_out).unwrap();
    assert_eq!(report.client_recovery_mode, "static_replica_everywhere");
    assert!(
        report
            .route_modes
            .iter()
            .all(|m| m.mode == "static_structural_replica")
    );
    assert!(report.client_recovery.is_none() && report.recovered_files.is_empty());
    let cli = |args: &[&str], out: &Path| {
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
            .args(["export", "--compatibility"])
            .arg(&root)
            .args(args)
            .arg("-o")
            .arg(out)
            .output()
            .unwrap();
        (
            run.status.success(),
            String::from_utf8_lossy(&run.stderr).into_owned(),
        )
    };
    let cli_static = temp.path().join("cli-static");
    assert!(cli(&["--no-recover-client"], &cli_static).0);
    assert_eq!(files(&static_out), files(&cli_static));
    // CLI default recovers and equals the library default; strict flag keeps its refusal.
    let cli_default = temp.path().join("cli-default");
    assert!(cli(&[], &cli_default).0);
    let lib_default = temp.path().join("lib-default");
    export::tanstack_start_recovering(&root, &lib_default, false).unwrap();
    assert_eq!(files(&cli_default), files(&lib_default));
    assert_ne!(files(&cli_default), files(&cli_static));
    let (ok, stderr) = cli(
        &["--recover-client-route", "/other"],
        &temp.path().join("cli-strict"),
    );
    assert!(!ok && !temp.path().join("cli-strict").exists(), "{stderr}");
    let (ok, _) = cli(
        &["--recover-client-route", "/catalog", "--no-recover-client"],
        &temp.path().join("cli-conflict"),
    );
    assert!(
        !ok,
        "--no-recover-client conflicts with --recover-client-route"
    );
}
#[test]
fn when_every_route_recovers_no_static_replica_claims_are_made() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    fixture(&root);
    project::build(&root).unwrap();
    let out = temp.path().join("project");
    let report = export::tanstack_start_recovering(&root, &out, false).unwrap();
    assert_eq!(report.runtime, "recovered_client_programs");
    assert!(
        report
            .route_modes
            .iter()
            .all(|m| m.mode == "recovered_client_program")
    );
    let limitations = report.limitations.join("\n");
    assert!(!limitations.contains("Animation is not replayed"));
    assert!(!limitations.contains("static structural replicas"));
    let readme = std::fs::read_to_string(out.join("README.md")).unwrap();
    assert!(!readme.contains("A **static structural replica** of what was captured"));
    assert!(readme.contains("1 of 1 captured route(s) run recovered original client code"));
    for vacuous in [
        "the rest are static replicas",
        "Static routes have no controllers",
        "on static routes they are PNG snapshots",
    ] {
        assert!(!readme.contains(vacuous), "{vacuous}");
    }
}
/// Three literal routes: `/a` and `/b` are native custom-element programs that
/// recover alone and together; `/c` has no client program at all.
fn native_routes(root: &Path) {
    source_fixture::fixture(root, 3, 1);
    for (route, path) in ["a", "b", "c"].iter().enumerate() {
        for relative in [
            format!("reconstruction/pages/{route}-0.json"),
            format!("reconstruction/manifests/{route}.json"),
        ] {
            let file = root.join(relative);
            let mut value: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            value["url"] = json!(format!("https://example.test/{path}"));
            std::fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        std::fs::create_dir_all(root.join(format!("site/{path}"))).unwrap();
    }
    for path in ["a", "b"] {
        std::fs::write(
            root.join(format!("site/{path}/index.html")),
            format!("<!doctype html><html><body><el-{path}>{path}</el-{path}><script type=module src=/{path}.js></script></body></html>"),
        )
        .unwrap();
        std::fs::write(
            root.join(format!("site/{path}.js")),
            format!("customElements.define('el-{path}',class extends HTMLElement{{}});"),
        )
        .unwrap();
    }
    std::fs::write(
        root.join("site/c/index.html"),
        "<!doctype html><html><body><p>static</p></body></html>",
    )
    .unwrap();
}
#[test]
fn default_recovery_does_not_drop_routes_that_only_conflict_when_merged_alone() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    native_routes(&root);
    let model = project::build(&root).unwrap();
    // The strict form recovers /a and /b together.
    let both = BTreeSet::from(["/a".into(), "/b".into()]);
    let strict = recover::plan(&root, &model, &both).unwrap();
    assert_eq!(strict.routes.len(), 2);
    // /c is refused, so the default cannot recover every route jointly; it must
    // still recover /a and /b, exactly as the strict selection of both does.
    let (recovery, fallbacks) = recover::plan_available(&root, &model).unwrap();
    let recovery = recovery.unwrap();
    assert_eq!(
        recovery.routes.keys().collect::<Vec<_>>(),
        ["/a", "/b"],
        "{fallbacks:?}"
    );
    assert_eq!(
        fallbacks
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["/c"]
    );
    assert_eq!(recovery.files, strict.files);
    assert_eq!(recovery.sources, strict.sources);
    recover::verify(&root, &recovery).unwrap();
}
#[test]
fn route_modes_list_every_captured_route_including_query_variants() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    // Two captured routes that differ only by query share the pathname /catalog.
    source_fixture::fixture(&root, 2, 1);
    project::build(&root).unwrap();
    let report = export::tanstack_start_recovering(&root, &temp.path().join("out"), false).unwrap();
    assert_eq!(report.routes, 2);
    assert_eq!(
        report
            .route_modes
            .iter()
            .map(|m| m.path.as_str())
            .collect::<Vec<_>>(),
        ["/catalog?kind=0&sort=asc", "/catalog?kind=1&sort=asc"]
    );
    assert!(report.route_modes.iter().all(|m| {
        m.reason
            .as_deref()
            .unwrap()
            .starts_with("fallback: client recovery refused")
    }));
}
#[test]
fn default_recovery_adds_routes_that_cannot_be_planned_together_one_at_a_time() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    native_routes(&root);
    // Each page alone stays under the 128 markup module targets; together they do not.
    for path in ["a", "b"] {
        let targets = (0..65)
            .map(|i| format!(" data-m{i}=\"/{path}{i}.js\""))
            .collect::<String>();
        std::fs::write(
            root.join(format!("site/{path}/index.html")),
            format!("<!doctype html><html><body><el-{path}{targets}>{path}</el-{path}><script type=module src=/{path}.js></script></body></html>"),
        )
        .unwrap();
    }
    let model = project::build(&root).unwrap();
    for path in ["/a", "/b"] {
        recover::plan(&root, &model, &BTreeSet::from([path.into()])).unwrap();
    }
    assert!(recover::plan(&root, &model, &BTreeSet::from(["/a".into(), "/b".into()])).is_err());
    let (recovery, fallbacks) = recover::plan_available(&root, &model).unwrap();
    assert_eq!(recovery.unwrap().routes.keys().collect::<Vec<_>>(), ["/a"]);
    assert_eq!(fallbacks.len(), 2);
    assert_eq!(fallbacks[0].path, "/b");
    assert!(
        fallbacks[0]
            .reason
            .starts_with("recoverable alone, but not together with /a: "),
        "{}",
        fallbacks[0].reason
    );
    assert_eq!(fallbacks[1].path, "/c");
    // The export publishes with the static fallbacks recorded.
    let report = export::tanstack_start_recovering(&root, &temp.path().join("out"), false).unwrap();
    let modes = report
        .route_modes
        .iter()
        .map(|m| (m.path.as_str(), m.mode.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        modes,
        [
            ("/a", "recovered_client_program"),
            ("/b", "static_structural_replica"),
            ("/c", "static_structural_replica")
        ]
    );
}
