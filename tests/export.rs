#![cfg(any(target_os = "macos", target_os = "linux"))]
use mochi::{export, project};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};
#[path = "support/source_fixture.rs"]
mod source_fixture;

fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
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
fn change(root: &Path, file: &str, edit: impl FnOnce(&mut Value)) {
    let path = root.join(file);
    let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    edit(&mut value);
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}
#[test]
fn emits_real_jsx_props_shared_layouts_literal_routes_and_reproducible_lockfile() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("capture");
    std::fs::create_dir(&source).unwrap();
    source_fixture::fixture(&source, 3, 2);
    for view in 0..2 {
        change(
            &source,
            &format!("reconstruction/pages/2-{view}.json"),
            |c| c["url"] = json!("https://example.test/products/item"),
        );
    }
    change(&source, "reconstruction/manifests/2.json", |m| {
        m["url"] = json!("https://example.test/products/item")
    });
    project::build(&source).unwrap();
    let original = files(&source);
    let first = root.path().join("first");
    let report = export::tanstack_start(&source, &first).unwrap();
    assert_eq!(report.routes, 3);
    assert!(report.components >= 3);
    assert_eq!(report.layouts, 1);
    assert_eq!(report.verification, "not_run");
    let output = files(&first);
    assert_eq!(
        original,
        files(&source),
        "export must not alter the source clone"
    );
    let jsx = output
        .iter()
        .filter(|(p, _)| p.starts_with("src/components/"))
        .map(|(_, v)| String::from_utf8(v.clone()).unwrap())
        .collect::<String>();
    assert!(jsx.contains("<article") && jsx.contains("props.prop_"));
    assert!(!jsx.contains("dangerouslySetInnerHTML"));
    assert!(!output.contains_key("src/route-config.ts"));
    assert!(output.contains_key("src/routes/products/item/index.tsx"));
    assert!(output.contains_key("src/features/pages/products/item/page.tsx"));
    assert!(output.contains_key(".mochi/export.json"));
    assert!(output.contains_key(".mochi/style-audit.json"));
    assert!(!output.contains_key("mochi.export.json"));
    let pages = output
        .iter()
        .filter(|(p, _)| p.starts_with("src/features/pages/"))
        .map(|(_, v)| String::from_utf8(v.clone()).unwrap())
        .collect::<String>();
    assert!(
        pages.contains("/catalog?kind=0&sort=asc") && pages.contains("/catalog?kind=1&sort=asc")
    );
    assert!(
        pages.contains("switch (page.key)"),
        "selection uses the literal route key, not metadata base href"
    );
    assert!(output.contains_key("bun.lock"));
    assert!(
        output
            .keys()
            .any(|p| p.starts_with("src/styles/compat/") && p.ends_with(".css"))
    );
    assert!(output.keys().all(|p| !p.ends_with("app.js")));
    assert!(
        report
            .diagnostics
            .contains_key("asset_ambiguous_mirror_path")
    );
    let second = root.path().join("second");
    export::tanstack_start(&source, &second).unwrap();
    assert_eq!(output, files(&second));
    let cli = root.path().join("cli");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .arg("export")
        .arg("--compatibility")
        .arg(&source)
        // Default CLI export recovers client programs per route; this fixture has none,
        // so every route falls back, but the report differs. Compare the static form.
        .args(["--no-recover-client", "--framework", "tanstack-start", "-o"])
        .arg(&cli)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(output, files(&cli));
}
#[test]
fn physical_route_case_collisions_refuse_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("capture");
    std::fs::create_dir(&source).unwrap();
    source_fixture::fixture(&source, 2, 1);
    for (index, path) in ["Catalog", "catalog"].iter().enumerate() {
        for file in [
            format!("reconstruction/pages/{index}-0.json"),
            format!("reconstruction/manifests/{index}.json"),
        ] {
            change(&source, &file, |v| {
                v["url"] = json!(format!("https://example.test/{path}"))
            });
        }
    }
    project::build(&source).unwrap();
    let out = temp.path().join("project");
    let error = export::tanstack_start(&source, &out).unwrap_err();
    assert!(error.to_string().contains("case-insensitive"), "{error}");
    assert!(!out.exists());
}
#[test]
fn stale_inputs_collisions_and_unsupported_mappings_never_publish_partial_projects() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("capture");
    std::fs::create_dir(&source).unwrap();
    source_fixture::fixture(&source, 1, 2);
    project::build(&source).unwrap();
    let out = root.path().join("out");
    std::fs::create_dir(&out).unwrap();
    std::fs::write(out.join("owned"), b"keep").unwrap();
    assert!(export::tanstack_start(&source, &out).is_err());
    assert_eq!(std::fs::read(out.join("owned")).unwrap(), b"keep");
    assert!(export::tanstack_start(&source, &source.join("nested")).is_err());
    let target = root.path().join("new");
    std::fs::write(source.join("site/brand.svg"), b"changed").unwrap();
    assert!(
        export::tanstack_start(&source, &target)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert!(!target.exists());
    source_fixture::fixture(&source, 1, 2);
    project::build(&source).unwrap();
    change(&source, "reconstruction/pages/0-1.json", |c| {
        c["nodes"][8]["value"] = json!("viewport-specific text")
    });
    project::build(&source).unwrap();
    export::tanstack_start(&source, &target).unwrap();
    let pages = walkdir::WalkDir::new(target.join("src/features/pages"))
        .into_iter()
        .map(|e| e.unwrap())
        .filter(|e| e.file_type().is_file())
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .collect::<String>();
    assert!(
        pages.contains("ResponsiveCapture widths=") && pages.contains("viewport-specific text")
    );
    let target = root.path().join("unsupported");
    source_fixture::fixture(&source, 1, 1);
    // Remove the second viewport from the committed inventory and its old file.
    std::fs::remove_file(source.join("reconstruction/pages/0-1.json")).unwrap();
    change(&source, "reconstruction/pages/0-0.json", |c| {
        c["stylesheets"][0]["rules"] = json!(["@import url('/other.css');"])
    });
    project::build(&source).unwrap();
    assert!(
        export::tanstack_start(&source, &target)
            .unwrap_err()
            .to_string()
            .contains("@import")
    );
    assert!(!target.exists());
    assert!(
        !std::fs::read_dir(root.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".mochi-export-")),
        "staging directories must be cleaned on failure"
    );
}
#[test]
fn executable_attributes_are_omitted_and_srcset_is_kept_and_reported() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("capture");
    std::fs::create_dir(&source).unwrap();
    source_fixture::fixture(&source, 1, 1);
    change(&source, "reconstruction/pages/0-0.json", |c| {
        c["nodes"][9]["attributes"].as_array_mut().unwrap().extend([
            json!({"name":"onerror","namespace":null,"value":"window.pwned=true"}),
            json!({"name":"srcset","namespace":null,"value":"/a.png 1x, /b.png 2x"}),
        ]);
        c["metadata"]["title"] = json!("Literal </title><script>window.pwned=true</script>");
    });
    project::build(&source).unwrap();
    let report = export::tanstack_start(&source, &root.path().join("out")).unwrap();
    assert!(
        report
            .diagnostics
            .contains_key("omitted_action_or_executable_attribute")
    );
    // srcset is no longer dropped by the exporter: the runtime parses, resolves
    // and filters its candidates, and the exporter reports where they point.
    assert!(!report.diagnostics.contains_key("srcset_not_reconstructed"));
    let jsx = files(&root.path().join("out"))
        .into_iter()
        .filter(|(p, _)| p.ends_with(".tsx"))
        .map(|(_, v)| String::from_utf8(v).unwrap())
        .collect::<String>();
    assert!(!jsx.contains("\"onerror\":"));
    assert!(jsx.contains("/a.png 1x, /b.png 2x"));
    assert!(
        report
            .external_references
            .iter()
            .any(|r| r.url.ends_with("/b.png"))
    );
}
#[test]
fn head_document_state_names_and_references_come_from_the_capture() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("capture");
    std::fs::create_dir(&source).unwrap();
    source_fixture::fixture(&source, 1, 2);
    for view in 0..2 {
        change(
            &source,
            &format!("reconstruction/pages/0-{view}.json"),
            |c| {
                c["url"] = json!("https://example.test/catalog");
                c["metadata"]["canonical"] = json!("https://example.test/catalog");
                c["metadata"]["meta"] = json!([
                    {"name":"description","property":null,"http_equiv":null,"content":"Catalog"},
                    {"name":null,"property":"csp-nonce","http_equiv":null,"content":"stale"},
                {"name":null,"property":"og:image","http_equiv":null,"content":"https://cdn.test/og.png"},
                    {"name":"next-size-adjust","property":null,"http_equiv":null,"content":""}
                ]);
                // Desktop-only runtime state on <body> (as a layout script would leave it).
                if view == 1 {
                    c["nodes"][1]["attributes"] =
                        json!([{"name":"style","namespace":null,"value":"padding-bottom: 99px;"}]);
                }
                for node in [6, 10] {
                    c["nodes"][node]["attributes"].as_array_mut().unwrap().push(
                        json!({"name":"data-component","namespace":null,"value":"product-card"}),
                    );
                }
                c["nodes"][3]["attributes"].as_array_mut().unwrap().push(
                    json!({"name":"ping","namespace":null,"value":"https://track.test/ping"}),
                );
                let nodes = c["nodes"].as_array_mut().unwrap();
                nodes.push(source_fixture::element(
                    Some(5),
                    "img",
                    &[
                        ("src", "data:image/svg+xml;base64,PHN2Zz48L3N2Zz4="),
                        ("alt", ""),
                    ],
                ));
                nodes.push(source_fixture::element(
                    Some(1),
                    "next-route-announcer",
                    &[],
                ));
                nodes.push(source_fixture::element(
                    Some(5),
                    "img",
                    &[("src", "https://cdn.test/hero.png"), ("alt", "Hero")],
                ));
            },
        );
    }
    change(&source, "reconstruction/manifests/0.json", |m| {
        m["url"] = json!("https://example.test/catalog")
    });
    std::fs::create_dir_all(source.join("site/catalog")).unwrap();
    std::fs::write(
        source.join("site/catalog/index.html"),
        r#"<html><head><link rel="icon" href="/favicon.png" type="image/png">
        <link rel="icon" href="/favicon.png" type="image/png" nonce="x">
        <link rel="apple-touch-icon" href="https://cdn.test/touch.png" sizes="180x180">
        <link rel="stylesheet" href="/a.css"><link rel="manifest" href="/m.webmanifest">
        <link rel="icon" href="data:text/html,%3Cscript%3Ex()%3C/script%3E">
        <link rel="icon" href="/scripted.svg"><link rel="icon" href="/plain.svg">
        <script type="application/ld+json">{"@type":"Organization","name":"</Example>","url":"u","logo":"l"}</script>
        <script>window.runtime = 1</script></head><body></body></html>"#,
    )
    .unwrap();
    std::fs::write(source.join("site/favicon.png"), b"icon bytes").unwrap();
    std::fs::write(
        source.join("site/scripted.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>",
    )
    .unwrap();
    std::fs::write(
        source.join("site/plain.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\"><path d=\"M0 0\"/></svg>",
    )
    .unwrap();
    project::build(&source).unwrap();
    let out = root.path().join("out");
    let report = export::tanstack_start(&source, &out).unwrap();
    let output = files(&out);
    let text = |path: &str| String::from_utf8(output[path].clone()).unwrap();
    let data = text("src/features/pages/catalog/data.ts");
    let page = text("src/features/pages/catalog/page.tsx");
    // Head: canonical, localized and remote icons, JSON-LD; no framework state.
    assert!(
        data.contains("'rel': 'canonical'") && data.contains("https://example.test/catalog"),
        "{data}"
    );
    assert!(data.contains("/_mochi/assets/") && data.contains("https://cdn.test/touch.png"));
    assert!(data.contains("application/ld+json") && data.contains("\\u003c/Example"));
    assert!(!data.contains("csp-nonce") && !data.contains("next-size-adjust"));
    // JSON-LD keeps the author's key order; non-image data: icons and scripted SVGs are not emitted.
    assert!(data.contains(r#""url":"u","logo":"l""#), "{data}");
    assert!(!data.contains("data:text/html"));
    assert!(report.diagnostics["asset_svg_not_copied"].details[0].contains("script element"));
    assert!(data.contains("https://example.test/scripted.svg") && !data.contains("alert"));
    assert!(
        output
            .keys()
            .any(|p| p.starts_with("public/_mochi/assets/") && p.ends_with(".svg"))
    );
    assert_eq!(
        report.head_inputs["site/catalog/index.html"],
        mochi::cache::fingerprint(&std::fs::read(source.join("site/catalog/index.html")).unwrap())
    );
    assert!(report.head_inputs.contains_key("site/favicon.png"));
    // `ping` is never emitted; inline SVG data images are no longer "unsupported".
    let sources = output
        .iter()
        .filter(|(p, _)| p.ends_with(".tsx"))
        .map(|(_, v)| String::from_utf8(v.clone()).unwrap())
        .collect::<String>();
    assert!(!sources.contains("'ping'") && !sources.contains("track.test"));
    assert!(
        report
            .diagnostics
            .contains_key("asset_inline_svg_data_image")
    );
    assert!(
        !report
            .diagnostics
            .contains_key("asset_inline_or_unsupported")
    );
    // Viewports that render identically are one element, not two copies.
    assert!(
        report
            .diagnostics
            .contains_key("identical_viewport_variants_merged")
    );
    assert!(!page.contains("ResponsiveCapture"));
    assert!(!data.contains("manifest") && !data.contains("a.css"));
    assert_eq!(
        data.matches("'rel': 'icon'").count(),
        3,
        "favicon.png (duplicate collapsed), scripted.svg (kept remote), plain.svg"
    );
    // Document state differs by viewport and is carried per viewport, not baked into all.
    assert!(data.contains("'documents': [") && data.contains("'body': {}"));
    assert_eq!(
        data.matches("padding-bottom").count(),
        2,
        "primary document + its variant only"
    );
    assert!(
        report
            .diagnostics
            .contains_key("document_attributes_differ_by_viewport")
    );
    // Framework runtime element is gone; naming follows data-component.
    assert!(
        !page.contains("next-route-announcer")
            && !report
                .diagnostics
                .contains_key("nonvisual_runtime_surface_excluded")
    );
    assert!(
        report
            .diagnostics
            .contains_key("framework_runtime_element_omitted")
    );
    assert!(output.contains_key("src/components/ProductCard.tsx"));
    // No unused locals: one CSS import in the page, none in data.ts, no unused helpers.
    assert_eq!(page.matches("?inline").count(), 1);
    assert!(!data.contains("?inline"));
    assert!(!text("src/components/ProductCard.tsx").contains("ShadowHost"));
    // References: every URL outside the export is listed with a kind.
    let kinds = |url: &str| {
        report
            .external_references
            .iter()
            .filter(|r| r.url == url)
            .flat_map(|r| r.kinds.iter().map(String::as_str))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kinds("https://example.test/products/0-a"),
        ["uncaptured_internal_link"]
    );
    assert!(kinds("https://cdn.test/hero.png").contains(&"external_resource"));
    assert_eq!(kinds("https://cdn.test/touch.png"), ["head_icon"]);
    // Emitted head URLs are listed once per URL, with every kind that reaches them.
    assert_eq!(kinds("https://cdn.test/og.png"), ["head_meta"]);
    assert!(kinds("https://example.test/catalog").contains(&"head_meta"));
    let urls: std::collections::BTreeSet<_> =
        report.external_references.iter().map(|r| &r.url).collect();
    assert_eq!(
        urls.len(),
        report.external_references.len(),
        "one row per URL"
    );
    // Requests seen only while capturing are a separate list, not "reached by the source".
    assert!(
        report
            .capture_time_requests
            .iter()
            .any(|r| r.url == "https://other.test/img.png")
    );
    assert!(kinds("https://other.test/img.png").is_empty());
    // local_assets counts exactly the files below public/.
    let public = output.keys().filter(|p| p.starts_with("public/")).count();
    assert_eq!(report.local_assets, public);
    assert!(public > 0);
    assert!(
        report.diagnostics["asset_runtime_observation"]
            .details
            .len()
            <= 8
    );
    assert!(report.diagnostics["head_icon_localized"].details.len() == 2);
    // One route entry emits one canonical link, although two viewports were captured.
    assert_eq!(report.diagnostics["head_canonical_emitted"].count, 1);
    // The report, README and EDITING.md describe a static replica and nothing else.
    for doc in ["README.md", "EDITING.md"] {
        let doc = text(doc);
        assert!(
            doc.contains("static structural replica")
                && doc.contains("no controllers or recovered runtime"),
            "{doc}"
        );
        assert!(!doc.contains("recovered client programs remain"));
        assert!(!doc.contains("`src/compat/recovered/`: formatted"));
    }
    assert_eq!(report.runtime, "static_structural_replica");
    assert!(
        report
            .limitations
            .iter()
            .any(|l| l.contains("Animation is not replayed"))
    );
}
#[test]
fn components_are_named_from_authored_bem_classes_unless_the_class_varies() {
    for (classes, expected) in [
        (
            ["product-list__item", "product-list__item"],
            Some("ProductListItem"),
        ),
        (
            [
                "product-list__item",
                "product-list__item product-list__item--hot",
            ],
            None,
        ),
        (["css-x1y2z3__root", "css-x1y2z3__root"], None),
        // Utility syntax is not a BEM block, although it contains `--`.
        (
            ["[--header:var(--nav)] card", "[--header:var(--nav)] card"],
            None,
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("capture");
        std::fs::create_dir(&source).unwrap();
        source_fixture::fixture(&source, 1, 1);
        change(&source, "reconstruction/pages/0-0.json", |c| {
            for (node, class) in [6, 10].into_iter().zip(classes) {
                c["nodes"][node]["attributes"] =
                    json!([{"name":"class","namespace":null,"value":class}]);
            }
        });
        project::build(&source).unwrap();
        let out = root.path().join("out");
        let report = export::tanstack_start(&source, &out).unwrap();
        let output = files(&out);
        match expected {
            Some(name) => {
                assert!(output.contains_key(&format!("src/components/{name}.tsx")));
                assert!(
                    report.diagnostics["component_named_from_markup"].details[0]
                        .contains("(class)")
                );
            }
            None => assert!(
                !output
                    .keys()
                    .any(|p| p.contains("ProductList") || p.contains("Css"))
                    && !report
                        .diagnostics
                        .contains_key("component_named_from_markup"),
                "{classes:?}: varying, generated or utility classes must not name components"
            ),
        }
    }
}
#[test]
fn template_ships_route_tree_generation_and_runtime_policy() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("capture");
    std::fs::create_dir(&source).unwrap();
    source_fixture::fixture(&source, 1, 1);
    project::build(&source).unwrap();
    let out = root.path().join("out");
    export::tanstack_start(&source, &out).unwrap();
    let output = files(&out);
    let text = |path: &str| String::from_utf8(output[path].clone()).unwrap();
    // A fresh checkout has no routeTree.gen.ts; `check` must generate it first,
    // and the file is committed (not ignored) like any TanStack Start project.
    assert!(text("package.json").contains(
        "\"check\": \"bun --bun scripts/generate-route-tree.ts && bun --bun tsc --noEmit\""
    ));
    assert!(text("package.json").contains(
        "\"clean\": \"rm -rf .output .tanstack .mochi/dist node_modules/.vite node_modules/.nitro node_modules/.vite-temp\""
    ));
    assert!(text("scripts/generate-route-tree.ts").contains("resolveConfig"));
    assert!(!text(".gitignore").contains("routeTree.gen.ts"));
    assert!(!output.contains_key("src/routeTree.gen.ts"));
    // The unit-tested policy module is emitted and used by the runtime.
    assert!(text("src/lib/policy.ts").contains("'inert'"));
    assert!(text("src/lib/policy.ts").contains("svgDataImage"));
    assert!(text("src/lib/runtime.tsx").contains("from './policy'"));
}
#[cfg(unix)]
#[test]
fn output_and_source_model_symlinks_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("capture");
    std::fs::create_dir(&source).unwrap();
    source_fixture::fixture(&source, 1, 1);
    project::build(&source).unwrap();
    let target = root.path().join("elsewhere");
    std::fs::create_dir(&target).unwrap();
    let out = root.path().join("out");
    std::os::unix::fs::symlink(&target, &out).unwrap();
    assert!(export::tanstack_start(&source, &out).is_err());
    let model = source.join("reconstruction/project.json");
    let saved = root.path().join("saved.json");
    std::fs::rename(&model, &saved).unwrap();
    std::os::unix::fs::symlink(saved, model).unwrap();
    assert!(export::tanstack_start(&source, &root.path().join("new")).is_err());
    assert!(std::fs::read_dir(target).unwrap().next().is_none());
}
#[test]
fn references_count_emitted_urls_and_names_never_come_from_varying_or_taken_values() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("capture");
    std::fs::create_dir(&source).unwrap();
    source_fixture::fixture(&source, 1, 1);
    change(&source, "reconstruction/pages/0-0.json", |c| {
        c["url"] = json!("https://example.test/catalog");
        let nodes = c["nodes"].as_array_mut().unwrap();
        // data-component differs between the two instances, so it is a prop.
        for (node, value) in [(6, "product-card"), (10, "blog-entry")] {
            nodes[node]["attributes"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name":"data-component","namespace":null,"value":value}));
        }
        nodes.push(source_fixture::element(
            Some(14),
            "img",
            &[
                ("srcset", "/s.png 100w 50h, /t.png .5x, /bad.png 1x 2x"),
                ("alt", "s"),
            ],
        ));
    });
    change(&source, "reconstruction/manifests/0.json", |m| {
        m["url"] = json!("https://example.test/catalog")
    });
    std::fs::create_dir_all(source.join("site/catalog")).unwrap();
    std::fs::write(
        source.join("site/catalog/index.html"),
        r#"<html><head><link rel="icon" href="data:,"><link rel="mask-icon" href="javascript:alert(1)" color="red"></head><body></body></html>"#,
    )
    .unwrap();
    project::build(&source).unwrap();
    let out = root.path().join("out");
    let report = export::tanstack_start(&source, &out).unwrap();
    let output = files(&out);
    let occurrences = |url: &str| {
        report
            .external_references
            .iter()
            .filter(|r| r.url == url)
            .map(|r| r.occurrences)
            .sum::<usize>()
    };
    // Each prop-bound URL is written once (one instance's prop), not again for the prototype.
    assert_eq!(occurrences("https://example.test/products/0-a"), 1);
    assert_eq!(occurrences("https://example.test/image.png?size=small"), 1);
    // srcset descriptors are not URLs; candidates the runtime drops are not references.
    assert_eq!(occurrences("https://example.test/s.png"), 1);
    assert_eq!(occurrences("https://example.test/t.png"), 1);
    for url in ["/50h", "/.5x", "/bad.png", "/100w"] {
        assert_eq!(
            occurrences(&format!("https://example.test{url}")),
            0,
            "{url}"
        );
    }
    // Icons: inline data stays and is not external; other schemes are not emitted.
    let data = String::from_utf8(output["src/features/pages/catalog/data.ts"].clone()).unwrap();
    assert!(
        data.contains("'href': 'data:,'") && !data.contains("javascript:"),
        "{data}"
    );
    assert!(
        report
            .external_references
            .iter()
            .all(|r| r.url.starts_with("https://"))
    );
    // A data-component value that varies per instance does not name the component.
    assert!(!output.keys().any(|p| p.contains("ProductCard")));
    assert!(
        !report
            .diagnostics
            .contains_key("component_named_from_markup")
    );
}
#[test]
fn recovered_exports_qualify_static_claims_and_avoid_recovered_component_names() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    source_fixture::fixture(&root, 2, 1);
    let recovered_url = "https://example.test/catalog/?kind=0&sort=asc";
    let client = format!(
        "Client{}",
        &mochi::cache::fingerprint(recovered_url.as_bytes())[..16]
    );
    std::fs::create_dir(root.join("site/catalog")).unwrap();
    std::fs::write(root.join("site/catalog/index.html"),r#"<!doctype html><html><body><live-counter><h1>Original text</h1></live-counter><script type="module" src="/entry.js"></script></body></html>"#).unwrap();
    std::fs::write(
        root.join("site/entry.js"),
        "customElements.define('live-counter',class extends HTMLElement{});",
    )
    .unwrap();
    for (route, url) in [(0, recovered_url), (1, "https://example.test/other")] {
        change(
            &root,
            &format!("reconstruction/pages/{route}-0.json"),
            |c| {
                c["url"] = json!(url);
                let nodes = c["nodes"].as_array_mut().unwrap();
                if route == 0 {
                    let mut node = source_fixture::element(Some(1), "live-counter", &[]);
                    node["bounds"]["width"] = json!(0);
                    node["bounds"]["height"] = json!(0);
                    nodes.push(node);
                } else {
                    // The static route's component asks for the recovered entry's name.
                    for node in [6, 10] {
                        nodes[node]["attributes"].as_array_mut().unwrap().push(
                        json!({"name":"data-component","namespace":null,"value":client.to_lowercase()}),
                    );
                    }
                }
            },
        );
        change(
            &root,
            &format!("reconstruction/manifests/{route}.json"),
            |m| m["url"] = json!(url),
        );
    }
    project::build(&root).unwrap();
    let out = temp.path().join("project");
    let report = export::tanstack_start_with_clients(
        &root,
        &out,
        false,
        &std::collections::BTreeSet::from(["/catalog/".into()]),
    )
    .unwrap();
    assert_eq!(
        report.client_recovery.as_ref().unwrap().routes["/catalog/"].component,
        client
    );
    assert!(out.join(format!("src/components/{client}.tsx")).exists());
    assert!(out.join(format!("src/components/{client}2.tsx")).exists());
    // Static-replica claims are scoped to static routes when programs were recovered.
    for limitation in &report.limitations {
        if limitation.contains("Animation is not replayed")
            || limitation.contains("static PNG snapshots")
            || limitation.contains("Buttons have no click handlers")
        {
            assert!(limitation.starts_with("On static routes"), "{limitation}");
        }
    }
    assert!(
        report
            .limitations
            .iter()
            .any(|l| l.starts_with("On recovered routes the capture's canvas"))
    );
}
#[test]
fn urls_written_into_sandboxed_frame_documents_are_reported() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("capture");
    std::fs::create_dir(&source).unwrap();
    source_fixture::fixture(&source, 1, 1);
    change(&source, "reconstruction/pages/0-0.json", |c| {
        let nodes = c["nodes"].as_array_mut().unwrap();
        let frame = nodes.len();
        nodes.push(source_fixture::element(
            Some(14),
            "iframe",
            &[("title", "f")],
        ));
        nodes.push(source_fixture::element(
            Some(frame),
            "img",
            &[("src", "https://cdn.test/framed.png")],
        ));
        nodes.push(source_fixture::element(
            Some(frame),
            "a",
            &[("href", "https://example.test/catalog?kind=0&sort=asc")],
        ));
        c["surfaces"] = json!({frame.to_string(): {"kind":"frame","mode":"open","css":[],
            "base_url":"https://example.test/","children":[frame + 1, frame + 2],"image":null,
            "bounds":{"x":0,"y":0,"width":20,"height":20}}});
    });
    project::build(&source).unwrap();
    let report = export::tanstack_start(&source, &root.path().join("out")).unwrap();
    let kinds = |url: &str| {
        report
            .external_references
            .iter()
            .filter(|r| r.url == url)
            .flat_map(|r| r.kinds.iter().map(String::as_str))
            .collect::<Vec<_>>()
    };
    assert!(kinds("https://cdn.test/framed.png").contains(&"external_resource"));
    // srcDoc links are not rewritten to the local route, so even a captured URL leaves the export.
    assert!(
        kinds("https://example.test/catalog?kind=0&sort=asc").contains(&"uncaptured_internal_link")
    );
    // A frame is re-hosted, not snapshotted; EDITING.md says which.
    let guide = std::fs::read_to_string(root.path().join("out/EDITING.md")).unwrap();
    assert!(
        guide.contains("0 static snapshot(s)") && guide.contains("1 sandboxed frame(s)"),
        "{guide}"
    );
}
#[test]
fn head_references_count_each_emitted_route_entry() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("capture");
    std::fs::create_dir(&source).unwrap();
    // Two query states of /catalog share one mirrored HTML response.
    source_fixture::fixture(&source, 2, 1);
    std::fs::create_dir_all(source.join("site/catalog")).unwrap();
    std::fs::write(
        source.join("site/catalog/index.html"),
        r#"<html><head><link rel="icon" href="https://cdn.test/i.png"></head></html>"#,
    )
    .unwrap();
    project::build(&source).unwrap();
    let report = export::tanstack_start(&source, &root.path().join("out")).unwrap();
    let icon = report
        .external_references
        .iter()
        .find(|r| r.url == "https://cdn.test/i.png")
        .unwrap();
    assert_eq!(
        (icon.kinds.as_slice(), icon.occurrences),
        (&["head_icon".to_string()][..], 2)
    );
}
#[test]
fn document_attribute_urls_relative_canonicals_and_refused_icons_are_reported() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("capture");
    std::fs::create_dir(&source).unwrap();
    source_fixture::fixture(&source, 1, 1);
    change(&source, "reconstruction/pages/0-0.json", |c| {
        c["metadata"]["canonical"] = json!("/catalog");
        c["nodes"][1]["attributes"] =
            json!([{"name":"background","namespace":null,"value":"https://cdn.test/bg.png"}]);
    });
    std::fs::create_dir_all(source.join("site/catalog")).unwrap();
    std::fs::write(
        source.join("site/catalog/index.html"),
        r#"<html><head><link rel="icon" href="/entity.svg">
        <script type="application/ld+json">{"name":"<!--</x>\u2028"}</script></head></html>"#,
    )
    .unwrap();
    // `&#x6a;avascript:` is `javascript:` to the XML parser that opens this file.
    std::fs::write(
        source.join("site/entity.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><a href=\"&#x6a;avascript:alert(1)\"><rect width=\"9\" height=\"9\"/></a></svg>",
    )
    .unwrap();
    project::build(&source).unwrap();
    let out = root.path().join("out");
    let report = export::tanstack_start(&source, &out).unwrap();
    let output = files(&out);
    let data = String::from_utf8(output["src/features/pages/catalog/data.ts"].clone()).unwrap();
    let kinds = |url: &str| {
        report
            .external_references
            .iter()
            .filter(|r| r.url == url)
            .flat_map(|r| r.kinds.clone())
            .collect::<Vec<_>>()
    };
    // <body background> is resolved and rendered by the runtime like any element URL.
    assert!(data.contains("https://cdn.test/bg.png"));
    assert_eq!(kinds("https://cdn.test/bg.png"), ["external_resource"]);
    // A relative canonical meant the original origin; it is emitted and listed resolved.
    assert!(
        data.contains("'href': 'https://example.test/catalog'"),
        "{data}"
    );
    assert_eq!(kinds("https://example.test/catalog"), ["head_meta"]);
    assert!(kinds("/catalog").is_empty());
    // The entity-encoded script URL keeps the SVG out of public/, and the file is hashed.
    assert!(report.diagnostics["asset_svg_not_copied"].details[0].contains("javascript: URL"));
    assert!(data.contains("https://example.test/entity.svg"));
    assert!(report.head_inputs.contains_key("site/entity.svg"));
    assert!(
        !output
            .values()
            .any(|bytes| bytes.windows(10).any(|w| w == b"&#x6a;avas"))
    );
    // JSON-LD reaches the page through dangerouslySetInnerHTML: the TS string must
    // keep `\u003c` escaped (two backslashes in source) so no `<` is ever emitted.
    assert!(data.contains(r"\\u003c!--\\u003c/x>"), "{data}");
    // One mirrored response serves every query state of the path, and says so.
    assert!(report.diagnostics.contains_key("head_source_ignores_query"));
}
