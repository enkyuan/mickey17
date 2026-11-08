use mochi::{
    cache::fingerprint,
    project::{self, Project, Target},
};
use serde_json::{Value, json};

#[path = "support/source_fixture.rs"]
mod source_fixture;
use source_fixture::{element, fixture};

fn descendants(capture: &mochi::evidence::Capture, root: usize) -> Vec<usize> {
    let mut tree = vec![Vec::new(); capture.nodes.len()];
    for (i, n) in capture.nodes.iter().enumerate() {
        if let Some(parent) = n.parent {
            tree[parent].push(i);
        }
    }
    let mut stack = vec![root];
    let mut out = Vec::new();
    while let Some(n) = stack.pop() {
        out.push(n);
        stack.extend(tree[n].iter().rev());
    }
    out
}
fn content_only(node: &mochi::evidence::Node) -> Value {
    let mut v = serde_json::to_value(node).unwrap();
    for key in ["parent", "style", "pseudo", "bounds"] {
        v.as_object_mut().unwrap().remove(key);
    }
    v
}

#[test]
fn model_preserves_evidence_and_reconstructs_content_from_props_deterministically() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), 2, 2);
    let model = project::build(root.path()).unwrap();
    assert!(
        !root.path().join("reconstruction/report.json").exists(),
        "model build is independent of report writes"
    );
    assert_eq!(model.routes.len(), 2);
    assert_eq!(model.observations.len(), 4);
    assert_eq!(model.styles.len(), 2);
    assert!(model.routes.values().all(|r|r.pathname=="/catalog" && r.query.as_deref().unwrap().ends_with("&sort=asc")));
    assert_eq!(model.layouts.len(), 2);
    assert!(model.layouts.iter().all(|l| l.occurrences.len() == 2));
    let captured_links: Vec<_> = model
        .navigation
        .iter()
        .filter(|n| n.href.ends_with("#items"))
        .collect();
    assert_eq!(captured_links.len(), 4);
    assert!(
        captured_links
            .iter()
            .all(|n| n.route.is_some() && n.resolved.as_ref().unwrap().ends_with("#items"))
    );
    assert!(
        model
            .navigation
            .iter()
            .filter(|n| n.href.starts_with("/products/"))
            .all(|n| n.route.is_none())
    );
    let cards = model
        .components
        .values()
        .find(|c| c.node_count == 4)
        .unwrap();
    assert_eq!(cards.logical_occurrences, 4);
    assert_eq!(cards.instances.len(), 8);
    assert_eq!(
        cards
            .instances
            .iter()
            .map(|i| &i.style_variant)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        2
    );
    assert!(cards.props.values().any(|p| p.target == Target::Text));
    assert!(
        cards
            .props
            .values()
            .any(|p| matches!(&p.target,Target::Attribute{name,..} if name=="href"))
    );
    assert!(
        cards
            .props
            .values()
            .any(|p| matches!(&p.target,Target::Attribute{name,..} if name=="src"))
    );
    let mut occupied = std::collections::BTreeSet::new();
    for component in model.components.values() {
        let prototype = &model.observations[&component.prototype.observation].capture;
        let prototype_nodes = descendants(prototype, component.prototype.node);
        for instance in &component.instances {
            let capture = &model.observations[&instance.source.observation].capture;
            let nodes = descendants(capture, instance.source.node);
            let mut restored: Vec<_> = prototype_nodes
                .iter()
                .map(|&n| content_only(&prototype.nodes[n]))
                .collect();
            for (name, prop) in &component.props {
                let value = &instance.props[name];
                let node = &mut restored[prop.relative_node];
                match &prop.target {
                    Target::Text => node["value"] = value.clone(),
                    Target::Control => node["control"] = value.clone(),
                    Target::Attribute { name, namespace } => {
                        let attr = node["attributes"]
                            .as_array_mut()
                            .unwrap()
                            .iter_mut()
                            .find(|a| {
                                a["name"] == *name
                                    && a["namespace"] == serde_json::to_value(namespace).unwrap()
                            })
                            .unwrap();
                        attr["value"] = value.clone();
                    }
                }
            }
            assert_eq!(
                restored,
                nodes
                    .iter()
                    .map(|&n| content_only(&capture.nodes[n]))
                    .collect::<Vec<_>>()
            );
            for n in nodes {
                assert!(
                    occupied.insert((&instance.source.observation, n)),
                    "overlapping extraction"
                );
            }
        }
    }
    let brand = &model.assets["https://example.test/brand.svg"];
    assert_eq!(brand.status, "local_candidate");
    assert_eq!(
        brand.local_sha256.as_deref(),
        Some(fingerprint(b"<svg/>").as_str())
    );
    assert_eq!(
        model.assets["https://example.test/image.png?size=small"].status,
        "ambiguous_mirror_path"
    );
    assert_eq!(
        model.assets["https://example.test/image.png?size=large"].status,
        "ambiguous_mirror_path"
    );
    assert_eq!(
        model.assets["https://other.test/img.png"].status,
        "external"
    );
    assert_eq!(
        model.assets["https://example.test/app.js"].status,
        "runtime_observation"
    );
    assert_eq!(
        model.assets["https://example.test/font.woff2"].status,
        "local_candidate"
    );
    assert!(model.observations.values().all(
        |o| !o.capture.fonts.is_empty() && o.capture.stylesheets[0].rules[0].contains("@media")
    ));
    let path = root.path().join("reconstruction/project.json");
    let first = std::fs::read(&path).unwrap();
    let saved: Project = serde_json::from_slice(&first).unwrap();
    saved.validate().unwrap();
    project::build(root.path()).unwrap();
    assert_eq!(first, std::fs::read(&path).unwrap());
    let cli = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .arg("model-project")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert_eq!(first, std::fs::read(path).unwrap());
}

#[test]
fn viewports_alone_do_not_establish_reuse_and_input_changes_invalidate_model() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), 1, 2);
    // Inserting a sibling moves the header/footer paths at another viewport.
    // Those are still single occurrences, not evidence for reusable components.
    let source = root.path().join("reconstruction/pages/0-1.json");
    let mut capture: Value = serde_json::from_slice(&std::fs::read(&source).unwrap()).unwrap();
    let nodes = capture["nodes"].as_array_mut().unwrap();
    for node in nodes.iter_mut() {
        if let Some(parent) = node["parent"].as_u64()
            && parent >= 2
        {
            node["parent"] = json!(parent + 1);
        }
    }
    nodes.insert(2, element(Some(1), "aside", &[]));
    std::fs::write(source, serde_json::to_vec(&capture).unwrap()).unwrap();
    let model = project::build(root.path()).unwrap();
    assert!(model.layouts.is_empty());
    assert!(
        model
            .components
            .values()
            .all(|c| c.node_count == 4 && c.logical_occurrences == 2)
    );
    let before = model.input_sha256;
    std::fs::write(root.path().join("site/brand.svg"), b"<svg>changed</svg>").unwrap();
    assert_ne!(project::build(root.path()).unwrap().input_sha256, before);
}

#[test]
fn invalid_or_incomplete_inputs_preserve_previous_model_and_invalid_references_fail() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), 2, 1);
    project::build(root.path()).unwrap();
    let path = root.path().join("reconstruction/project.json");
    let bytes = std::fs::read(&path).unwrap();
    let raw: Value = serde_json::from_slice(&bytes).unwrap();
    for case in 0..7 {
        let mut bad = raw.clone();
        let observation = bad["observations"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        let component = bad["components"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        match case {
            0 => bad["schema_version"] = json!(99),
            1 => bad["observations"][&observation]["style_ids"][0] = json!("absent"),
            2 => bad["components"][&component]["prototype"]["node"] = json!(9999),
            3 => {
                bad["observations"][&observation]["capture"]["metadata"]["title"] = json!("changed")
            }
            4 => bad["layouts"][0]["occurrences"][0]["main"] = json!(9999),
            5 => {
                bad["assets"]["https://example.test/brand.svg"]["local_path"] = json!("../outside")
            }
            _ => bad["navigation"][0]["route"] = json!("missing"),
        }
        assert!(
            serde_json::from_value::<Project>(bad)
                .unwrap()
                .validate()
                .is_err(),
            "case {case}"
        );
    }
    let source = root.path().join("reconstruction/pages/0-0.json");
    let mut capture: Value = serde_json::from_slice(&std::fs::read(&source).unwrap()).unwrap();
    capture["complete"] = json!(false);
    capture["issues"] = json!([{"code":"stylesheet_unavailable","detail":"blocked","node":null}]);
    std::fs::write(&source, serde_json::to_vec(&capture).unwrap()).unwrap();
    assert!(
        project::build(root.path())
            .unwrap_err()
            .to_string()
            .contains("incomplete")
    );
    assert_eq!(bytes, std::fs::read(&path).unwrap());
    fixture(root.path(), 2, 1);
    std::fs::write(
        root.path().join("reconstruction/screens/0-0.png"),
        b"changed",
    )
    .unwrap();
    assert!(project::build(root.path()).is_err());
    assert_eq!(bytes, std::fs::read(path).unwrap());
    fixture(root.path(), 2, 1);
    std::fs::OpenOptions::new()
        .write(true)
        .open(root.path().join("site/brand.svg"))
        .unwrap()
        .set_len(mochi::mirror::MAX_BODY + 1)
        .unwrap();
    assert!(
        project::build(root.path())
            .unwrap_err()
            .to_string()
            .contains("byte budget")
    );
    assert_eq!(
        bytes,
        std::fs::read(root.path().join("reconstruction/project.json")).unwrap()
    );
}

#[test]
fn incompatible_semantic_attributes_prevent_component_alignment() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), 1, 1);
    let source = root.path().join("reconstruction/pages/0-0.json");
    let mut capture: Value = serde_json::from_slice(&std::fs::read(&source).unwrap()).unwrap();
    for (node, role) in [(6, "listitem"), (10, "button")] {
        capture["nodes"][node]["attributes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"role","namespace":null,"value":role}));
    }
    std::fs::write(source, serde_json::to_vec(&capture).unwrap()).unwrap();
    assert!(project::build(root.path()).unwrap().components.is_empty());
}

#[cfg(unix)]
#[test]
fn local_asset_and_output_symlinks_are_refused() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), 1, 1);
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("secret");
    std::fs::write(&target, b"untouched").unwrap();
    std::fs::remove_file(root.path().join("site/brand.svg")).unwrap();
    std::os::unix::fs::symlink(&target, root.path().join("site/brand.svg")).unwrap();
    assert!(project::build(root.path()).is_err());
    std::fs::remove_file(root.path().join("site/brand.svg")).unwrap();
    fixture(root.path(), 1, 1);
    std::os::unix::fs::symlink(&target, root.path().join("reconstruction/project.json")).unwrap();
    assert!(project::build(root.path()).is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"untouched");
}
