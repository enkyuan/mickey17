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
        .args(["--framework", "tanstack-start", "-o"])
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
fn executable_attributes_and_srcset_are_omitted_and_reported() {
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
    assert!(report.diagnostics.contains_key("srcset_not_reconstructed"));
    let jsx = files(&root.path().join("out"))
        .into_iter()
        .filter(|(p, _)| p.ends_with(".tsx"))
        .map(|(_, v)| String::from_utf8(v).unwrap())
        .collect::<String>();
    assert!(!jsx.contains("\"onerror\":"));
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
