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
