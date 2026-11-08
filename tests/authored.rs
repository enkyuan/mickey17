#[path = "support/authored_fixture.rs"]
mod support;
use mochi::authored;
use serde_json::json;
#[test]
fn semantic_components_replace_controllers_and_keep_typed_routes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    let spec = support::fixture(&root);
    let input = temp.path().join("spec.json");
    std::fs::write(&input, serde_json::to_vec(&spec).unwrap()).unwrap();
    let out = temp.path().join("app");
    let report = authored::generate(&root, &input, &out).unwrap();
    assert_eq!(report.recovered_modules, 0);
    assert_eq!(report.components, 5);
    assert_eq!(report.verification, "not_run");
    assert!(!out.join("src/compat").exists());
    assert!(!out.join("src/lib/runtime.ts").exists());
    let nav = std::fs::read_to_string(out.join("src/components/compositions/site-navigation.tsx"))
        .unwrap();
    assert!(nav.contains("@/components/ui/navigation-menu"));
    assert!(nav.contains("NavigationMenu.Viewport"));
    assert!(!out.join("src/components/ui/site-navigation.tsx").exists());
    let primitive =
        std::fs::read_to_string(out.join("src/components/ui/navigation-menu.tsx")).unwrap();
    assert!(primitive.contains("export { NavigationMenu }"));
    assert!(primitive.contains("@base-ui/react/navigation-menu"));
    assert!(!out.join("src/components/ui/tabs.tsx").exists());
    assert!(nav.contains("SiteNavigationProps"));
    assert!(out.join("src/routes/catalog/index.tsx").exists());
    assert!(authored::generate(&root, &input, &out).is_err());
}
#[test]
fn invalid_semantic_programs_fail_before_publishing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    let base = support::fixture(&root);
    let input = temp.path().join("spec.json");
    let out = temp.path().join("app");
    let mutations = [
        ("/routes/0/props/links/1/href", json!("/catalog")),
        ("/input_sha256", json!("stale")),
        ("/routes/0/props/heading", json!(42)),
        (
            "/components/CatalogPage/body",
            json!({"kind":"component","name":"CatalogPage","props":{"heading":{"kind":"literal","value":"loop"},"links":{"kind":"literal","value":[]}}}),
        ),
        (
            "/components/ContactForm/body/props/onSubmit/prevent_default",
            json!(false),
        ),
        (
            "/components/ContactForm/body/children/0/children",
            json!([]),
        ),
        ("/components/SiteNavigation/body/children", json!([])),
        ("/tokens/ink", json!("var(--missing)")),
        ("/tokens/ink", json!("var(--surface)")),
        (
            "/components/CatalogPage/body/props",
            json!({"dangerouslySetInnerHTML":{"kind":"literal","value":"<script />"}}),
        ),
    ];
    for (pointer, value) in mutations {
        let mut spec = base.clone();
        *spec.pointer_mut(pointer).unwrap() = value;
        if pointer == "/tokens/ink" && spec["tokens"]["ink"] == "var(--surface)" {
            spec["tokens"]["surface"] = "var(--ink)".into();
        }
        std::fs::write(&input, serde_json::to_vec(&spec).unwrap()).unwrap();
        assert!(
            authored::generate(&root, &input, &out).is_err(),
            "accepted {pointer}"
        );
        assert!(!out.exists());
    }
}

#[test]
fn literal_assets_are_localized_and_plan_is_not_an_authored_program() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    let mut spec = support::fixture(&root);
    spec["assets"]["brand"] = "https://example.test/brand.svg".into();
    spec["components"]["CatalogPage"]["body"]["children"].as_array_mut().unwrap().push(json!({"kind":"element","tag":"img","props":{"src":{"kind":"literal","value":"brand"},"alt":{"kind":"literal","value":"Brand"}},"children":[],"classes":[]}));
    let input = temp.path().join("spec.json");
    std::fs::write(&input, serde_json::to_vec(&spec).unwrap()).unwrap();
    let out = temp.path().join("app");
    authored::generate(&root, &input, &out).unwrap();
    let page = std::fs::read_to_string(out.join("src/features/catalog/catalog-page.tsx")).unwrap();
    assert!(page.contains("assets.brand"));
    assert!(out.join("src/assets/images/brand.svg").exists());
    assert!(
        std::fs::read_to_string(out.join("src/lib/assets.ts"))
            .unwrap()
            .contains("@/assets/images/brand.svg")
    );
    assert!(!page.contains("useState"));
    assert!(page.contains("import { assets }"));
    let plan = temp.path().join("plan.json");
    authored::plan(&root, &plan).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&plan).unwrap()).unwrap();
    assert_eq!(value["status"], "semantic_decisions_required");
    assert!(authored::plan(&root, &plan).is_err());
    assert!(authored::generate(&root, &plan, &temp.path().join("not-an-app")).is_err());
}

#[test]
fn owned_graphics_are_typed_reachable_modules_with_explicit_imports() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    let mut spec = support::fixture(&root);
    let evidence = spec["components"]["CatalogPage"]["evidence"].clone();
    spec["source_modules"] = json!({
        "features/catalog/graphics/catalog-art.tsx": {"file":"art.tsx","purpose":"Owned procedural artwork","evidence":evidence},
        "assets/shaders/catalog.glsl": {"file":"catalog.glsl","purpose":"Editable fragment shader","evidence":evidence}
    });
    spec["external_components"] = json!({"CatalogArt":{"module":"features/catalog/graphics/catalog-art.tsx","props":{"label":{"type":"text"}}}});
    spec["components"]["CatalogPage"]["body"]["children"].as_array_mut().unwrap().push(json!({"kind":"external","name":"CatalogArt","props":{"label":{"kind":"literal","value":"Artwork"}}}));
    let source = "import shader from '@/assets/shaders/catalog.glsl?raw';\nexport function CatalogArt({label}:{label:string}) {return <canvas aria-label={label} data-shader-length={shader.length} /> }\n";
    std::fs::write(temp.path().join("art.tsx"), source).unwrap();
    std::fs::write(
        temp.path().join("catalog.glsl"),
        "void main() { gl_FragColor = vec4(1.0); }\n",
    )
    .unwrap();
    let input = temp.path().join("spec.json");
    std::fs::write(&input, serde_json::to_vec(&spec).unwrap()).unwrap();
    let out = temp.path().join("app");
    let report = authored::generate(&root, &input, &out).unwrap();
    assert_eq!(report.source_modules.len(), 2);
    assert!(out.join("src/features/catalog/catalog-content.ts").exists());
    assert!(out.join("src/assets/shaders/catalog.glsl").exists());
    let bad = temp.path().join("bad");
    for invalid in [
        source.replace("CatalogArt", "WrongExport"),
        source.replace("catalog.glsl", "missing.glsl"),
        source.replace("label:string", "label:any"),
    ] {
        std::fs::write(temp.path().join("art.tsx"), invalid).unwrap();
        assert!(authored::generate(&root, &input, &bad).is_err());
        assert!(!bad.exists());
    }
    std::fs::write(temp.path().join("art.tsx"), source).unwrap();
    let mut reserved = spec.clone();
    let module = reserved["source_modules"]
        .as_object_mut()
        .unwrap()
        .remove("features/catalog/graphics/catalog-art.tsx")
        .unwrap();
    reserved["source_modules"]["components/ui/catalog-art.tsx"] = module;
    reserved["external_components"]["CatalogArt"]["module"] =
        json!("components/ui/catalog-art.tsx");
    std::fs::write(&input, serde_json::to_vec(&reserved).unwrap()).unwrap();
    assert!(authored::generate(&root, &input, &bad).is_err());
    assert!(!bad.exists());
    spec["source_modules"]["features/catalog/orphan.ts"] =
        json!({"file":"orphan.ts","purpose":"Unused code","evidence":evidence});
    std::fs::write(temp.path().join("orphan.ts"), "export const orphan=1").unwrap();
    std::fs::write(&input, serde_json::to_vec(&spec).unwrap()).unwrap();
    assert!(authored::generate(&root, &input, &bad).is_err());
}

#[test]
fn reviewed_local_assets_and_styles_have_closed_import_graphs() {
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    let mut spec = support::fixture(&root);
    let evidence = spec["components"]["CatalogPage"]["evidence"].clone();
    let picture =
        b"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 1 1'><circle r='1'/></svg>";
    let source = "import image from '@/assets/vectors/catalog.svg'; import '@/styles/catalog-fonts.css'; import {Accordion} from '@/components/ui/accordion'; export function CatalogArt(){return <Accordion.Root><img src={image} alt='Catalog'/></Accordion.Root>}";
    std::fs::write(temp.path().join("art.tsx"), source).unwrap();
    std::fs::write(temp.path().join("catalog.svg"), picture).unwrap();
    std::fs::write(
        temp.path().join("theme.css"),
        ":root { --catalog-ink: black; }",
    )
    .unwrap();
    spec["source_modules"] = json!({
        "features/catalog/catalog-art.tsx":{"file":"art.tsx","purpose":"Catalog artwork","evidence":evidence},
        "styles/catalog-fonts.css":{"file":"theme.css","purpose":"Catalog theme","evidence":evidence}
    });
    spec["source_assets"] = json!({"assets/vectors/catalog.svg":{"file":"catalog.svg","sha256":format!("{:x}",Sha256::digest(picture)),"purpose":"Owned catalog vector","evidence":evidence}});
    spec["external_components"] =
        json!({"CatalogArt":{"module":"features/catalog/catalog-art.tsx"}});
    spec["components"]["CatalogPage"]["body"]["children"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"external","name":"CatalogArt"}));
    let input = temp.path().join("spec.json");
    std::fs::write(&input, serde_json::to_vec(&spec).unwrap()).unwrap();
    let out = temp.path().join("app");
    let report = authored::generate(&root, &input, &out).unwrap();
    assert_eq!(report.source_assets.len(), 1);
    assert_eq!(
        std::fs::read(out.join("src/assets/vectors/catalog.svg")).unwrap(),
        picture
    );
    assert!(out.join("src/components/ui/accordion.tsx").exists());
    let bad = temp.path().join("bad");
    std::fs::write(temp.path().join("catalog.svg"), b"changed").unwrap();
    assert!(
        authored::generate(&root, &input, &bad)
            .unwrap_err()
            .to_string()
            .contains("hash mismatch")
    );
    assert!(!bad.exists());
    std::fs::write(temp.path().join("catalog.svg"), picture).unwrap();
    std::fs::write(
        temp.path().join("art.tsx"),
        source.replace(
            "import image from '@/assets/vectors/catalog.svg';",
            "const image='';",
        ),
    )
    .unwrap();
    assert!(
        authored::generate(&root, &input, &bad)
            .unwrap_err()
            .to_string()
            .contains("unreachable")
    );
    assert!(!bad.exists());
    std::fs::write(temp.path().join("art.tsx"), source).unwrap();
    std::fs::write(
        temp.path().join("theme.css"),
        ".art{background:url(https://example.test/remote.svg)}",
    )
    .unwrap();
    assert!(authored::generate(&root, &input, &bad).is_err());
    assert!(!bad.exists());
}

#[test]
fn cli_requires_an_explicit_compatibility_choice() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("app");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .args(["export", "missing-capture", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("--authored-spec"));
    assert!(!out.exists());
}

#[test]
fn owned_pages_use_direct_file_routes_and_single_design_definitions() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    let mut spec = support::fixture(&root);
    let evidence = spec["components"]["CatalogPage"]["evidence"].clone();
    spec["components"] = json!({"CatalogPage":{"feature":"catalog","purpose":"Catalog page","evidence":evidence,"body":{"kind":"external","name":"CatalogScreen"}}});
    spec["routes"][0]["props"] = json!({});
    spec["tokens"] = json!({});
    spec["token_scopes"] = json!([]);
    spec["globals"] = json!([]);
    spec["source_modules"] = json!({
        "features/catalog/catalog-screen.tsx":{"file":"screen.tsx","purpose":"Typed catalog page","evidence":evidence},
        "styles/tokens.css":{"file":"tokens.css","purpose":"Single owned token definition","evidence":evidence}
    });
    spec["external_components"] =
        json!({"CatalogScreen":{"module":"features/catalog/catalog-screen.tsx"}});
    std::fs::write(temp.path().join("screen.tsx"),"import '@/styles/tokens.css'; export function CatalogScreen(){return <main><h1>Catalog</h1></main>}").unwrap();
    std::fs::write(temp.path().join("tokens.css"), ":root { --ink: #123; }").unwrap();
    let input = temp.path().join("spec.json");
    std::fs::write(&input, serde_json::to_vec(&spec).unwrap()).unwrap();
    let out = temp.path().join("app");
    authored::generate(&root, &input, &out).unwrap();
    let route = std::fs::read_to_string(out.join("src/routes/catalog/index.tsx")).unwrap();
    assert!(route.contains("component: CatalogScreen"));
    assert!(!out.join("src/features/catalog/catalog-page.tsx").exists());
    assert!(!out.join("src/features/catalog/catalog-content.ts").exists());
    assert!(!out.join("src/lib/assets.ts").exists());
    assert!(
        std::fs::read_to_string(out.join("src/styles/tokens.css"))
            .unwrap()
            .contains("#123")
    );
    spec["tokens"] = json!({"ink":"black"});
    std::fs::write(&input, serde_json::to_vec(&spec).unwrap()).unwrap();
    assert!(authored::generate(&root, &input, &temp.path().join("bad")).is_err());
}
