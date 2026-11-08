use mochi::{cache::fingerprint, evidence};
use serde_json::{Value, json};
use std::path::Path;

fn element(parent: Value, tag: &str) -> Value {
    json!({"parent":parent,"kind":"element","tag":tag,"namespace":"http://www.w3.org/1999/xhtml",
        "attributes":[],"style":0,"pseudo":{},"control":null,"bounds":{"x":0,"y":0,"width":200,"height":20}})
}
fn fixture() -> Value {
    json!({"schema_version":1,"url":"https://example.test/?variant=one",
        "metadata":{"title":"A page","base_url":"https://example.test/","lang":"en","doctype":"html","canonical":null,"meta":[]},
        "viewport":{"width":200,"height":200},"color_scheme":"light","state":"initial","scope":"light-dom-cssom-before-after",
        "nodes":[element(Value::Null,"html"),element(json!(0),"body"),
            {"parent":1,"kind":"text","value":"before ".repeat(200)},element(json!(1),"em"),
            {"parent":3,"kind":"text","value":"inside 🙂"},{"parent":1,"kind":"text","value":" after 🙂"}],
        "styles":[{"display":"block","font-size":"20px","--brand":"#123456"}],
        "stylesheets":[{"href":null,"base_url":"https://example.test/","disabled":false,"media":"","rules":["@media (max-width: 500px) { body { color: red; } }"],"readable":true}],
        "fonts":[{"stylesheet":0,"descriptors":{"font-family":"Fixture","font-display":"swap","src":"url(/font.woff2)"}}],
        "resources":[],"screenshot":{"path":"reconstruction/screens/fixture.png","sha256":fingerprint(include_bytes!("fixtures/source-reference.png"))},
        "complete":true,"issues":[]})
}
fn save(root: &Path, value: &Value) {
    std::fs::create_dir_all(root.join("reconstruction/pages")).unwrap();
    std::fs::create_dir_all(root.join("reconstruction/screens")).unwrap();
    std::fs::create_dir_all(root.join("reconstruction/manifests")).unwrap();
    std::fs::write(root.join("reconstruction/manifests/one.json"), serde_json::to_vec(&json!({"schema_version":1,"url":value["url"],"viewports":[value["viewport"].clone()],"snapshots":["reconstruction/pages/one.json"]})).unwrap()).unwrap();
    std::fs::write(
        root.join("reconstruction/pages/one.json"),
        serde_json::to_vec(value).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("reconstruction/screens/fixture.png"),
        include_bytes!("fixtures/source-reference.png"),
    )
    .unwrap();
}

#[test]
fn full_content_survives_and_legacy_projection_preserves_mixed_text_positions() {
    let root = tempfile::tempdir().unwrap();
    let mut value = fixture();
    for i in 0..50 {
        value["nodes"][1]["attributes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":format!("data-{i}"),"namespace":null,"value":"custom"}));
    }
    save(root.path(), &value);
    let capture = evidence::load(root.path(), Path::new("reconstruction/pages/one.json")).unwrap();
    capture.require_complete().unwrap();
    assert!(matches!(&capture.nodes[2].content,evidence::Content::Text{value} if value.len()>512));
    let projected = capture.to_analysis_snapshot().unwrap();
    assert_eq!(projected.nodes[0].text_slots, Some(vec![0, 1]));
    assert_eq!(projected.nodes[1].parent, Some(0));
    assert!(projected.nodes[0].attributes.is_empty());
    assert!(projected.nodes[0].text.encode_utf16().count() <= 512);
    let analysis = root.path().join("analysis/pages");
    std::fs::create_dir_all(&analysis).unwrap();
    std::fs::write(
        analysis.join("one.json"),
        serde_json::to_vec(&projected).unwrap(),
    )
    .unwrap();
    assert_eq!(
        mochi::analysis::analyze(root.path())
            .unwrap()
            .templates
            .len(),
        1
    );
    assert_eq!(capture.styles[0]["--brand"], "#123456");
    assert!(capture.stylesheets[0].rules[0].contains("@media"));
    assert_eq!(capture.fonts[0].descriptors["font-display"], "swap");
    let first = evidence::inspect(root.path()).unwrap();
    assert_eq!(first.complete, 1);
    let report = std::fs::read(root.path().join("reconstruction/report.json")).unwrap();
    std::fs::remove_file(root.path().join("reconstruction/pages/one.json")).unwrap();
    assert!(evidence::inspect(root.path()).is_err());
    assert_eq!(
        report,
        std::fs::read(root.path().join("reconstruction/report.json")).unwrap()
    );
    save(root.path(), &value);
    std::fs::write(root.path().join("reconstruction/pages/orphan.json"), b"{}").unwrap();
    assert!(
        evidence::inspect(root.path())
            .unwrap_err()
            .to_string()
            .contains("no manifest")
    );
    std::fs::remove_file(root.path().join("reconstruction/pages/orphan.json")).unwrap();
    std::fs::write(root.path().join("manifest.json"), serde_json::to_vec(&json!({"pages":[{"source_evidence":["reconstruction/pages/one.json","reconstruction/pages/missing-page.json"]}]})).unwrap()).unwrap();
    assert!(
        evidence::inspect(root.path())
            .unwrap_err()
            .to_string()
            .contains("inventory")
    );
    std::fs::remove_file(root.path().join("manifest.json")).unwrap();
    evidence::inspect(root.path()).unwrap();
    assert_eq!(
        report,
        std::fs::read(root.path().join("reconstruction/report.json")).unwrap()
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .arg("check-evidence")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn invalid_evidence_and_changed_screenshots_preserve_the_previous_report() {
    let root = tempfile::tempdir().unwrap();
    save(root.path(), &fixture());
    evidence::inspect(root.path()).unwrap();
    let report = std::fs::read(root.path().join("reconstruction/report.json")).unwrap();
    for case in 0..12 {
        let mut bad = fixture();
        match case {
            0 => bad["schema_version"] = json!(2),
            1 => bad["nodes"][1]["parent"] = json!(1),
            2 => bad["nodes"][3]["parent"] = json!(2),
            3 => bad["nodes"][1]["style"] = json!(99),
            4 => bad["nodes"][1]["pseudo"] = json!({"::before":99}),
            5 => bad["fonts"][0]["stylesheet"] = json!(9),
            6 => bad["issues"] = json!([{"code":"node_limit","detail":"partial","node":null}]),
            7 => bad["screenshot"]["path"] = json!("reconstruction/screens/../outside.png"),
            8 => bad["url"] = json!("file:///private/input"),
            9 => bad["nodes"][1]["bounds"]["width"] = json!(-1),
            10 => bad["viewport"]["width"] = json!(375),
            _ => {
                bad["resources"] =
                    json!([{"url":"/image.png","initiator":"img","response_status":1000}])
            }
        }
        save(root.path(), &bad);
        assert!(evidence::inspect(root.path()).is_err(), "case {case}");
        assert_eq!(
            report,
            std::fs::read(root.path().join("reconstruction/report.json")).unwrap()
        );
    }
    save(root.path(), &fixture());
    std::fs::write(
        root.path().join("reconstruction/screens/fixture.png"),
        b"changed",
    )
    .unwrap();
    assert!(
        evidence::inspect(root.path())
            .unwrap_err()
            .to_string()
            .contains("digest")
    );
    std::fs::write(
        root.path().join("reconstruction/pages/one.json"),
        vec![b' '; evidence::MAX_BYTES as usize + 1],
    )
    .unwrap();
    assert!(
        evidence::inspect(root.path())
            .unwrap_err()
            .to_string()
            .contains("byte limit")
    );
    assert_eq!(
        report,
        std::fs::read(root.path().join("reconstruction/report.json")).unwrap()
    );
}

#[test]
fn incomplete_evidence_is_inspectable_but_never_ready_for_projection_or_cli_validation() {
    let root = tempfile::tempdir().unwrap();
    let mut value = fixture();
    value["complete"] = json!(false);
    value["issues"] =
        json!([{"code":"stylesheet_unavailable","detail":"Remote CSSOM unavailable","node":null}]);
    value["stylesheets"][0]["readable"] = json!(false);
    save(root.path(), &value);
    assert_eq!(evidence::inspect(root.path()).unwrap().complete, 0);
    let capture = evidence::load(root.path(), Path::new("reconstruction/pages/one.json")).unwrap();
    assert!(capture.to_analysis_snapshot().is_err());
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .arg("check-evidence")
        .arg(root.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("incomplete"));
}

#[test]
fn source_options_fail_before_network_or_output_and_legacy_inputs_are_not_source_evidence() {
    use mochi::crawl;
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("clone");
    let viewport = evidence::Viewport {
        width: 375,
        height: 812,
    };
    for viewports in [
        vec![],
        vec![viewport.clone(); 2],
        vec![evidence::Viewport {
            width: 0,
            height: 812,
        }],
    ] {
        assert!(
            crawl::clone_with_evidence(
                "http://127.0.0.1:1",
                &out,
                &crawl::Options {
                    max_pages: 1,
                    extract: false,
                    keep_trackers: false,
                    ignore_robots: false
                },
                &crawl::PipelineOptions::default(),
                &evidence::CaptureOptions { viewports }
            )
            .is_err()
        );
        assert!(!out.exists());
    }
    for args in [
        vec!["--source-viewports", "375x812"],
        vec!["--source-evidence", "--source-viewports", "0x812"],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
            .args(["clone", "http://127.0.0.1:1"])
            .args(args)
            .arg("-o")
            .arg(&out)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(!out.exists());
    }
    std::fs::create_dir_all(root.path().join("analysis/pages")).unwrap();
    assert!(
        evidence::inspect(root.path())
            .unwrap_err()
            .to_string()
            .contains("--source-evidence")
    );
}

#[cfg(unix)]
#[test]
fn symlinked_evidence_or_screenshot_cannot_escape_the_clone() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    save(root.path(), &fixture());
    std::fs::write(outside.path().join("keep"), "untouched").unwrap();
    let png = root.path().join("reconstruction/screens/fixture.png");
    std::fs::remove_file(&png).unwrap();
    std::os::unix::fs::symlink(outside.path().join("keep"), &png).unwrap();
    assert!(evidence::inspect(root.path()).is_err());
    assert_eq!(
        std::fs::read_to_string(outside.path().join("keep")).unwrap(),
        "untouched"
    );
    std::fs::remove_file(&png).unwrap();
    save(root.path(), &fixture());
    let input = root.path().join("reconstruction/pages/one.json");
    std::fs::remove_file(&input).unwrap();
    std::os::unix::fs::symlink(outside.path().join("keep"), &input).unwrap();
    assert!(evidence::inspect(root.path()).is_err());
}
