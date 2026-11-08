use mochi::parity::{Capture, Contract, compare, project_fingerprint};
use serde_json::{Value, json};
fn fixture() -> (Value, Value) {
    let mut states = serde_json::Map::new();
    let mut captures = serde_json::Map::new();
    for viewport in [[390, 844], [1440, 900]] {
        for (phase, reduced) in [
            ("initial", false),
            ("scroll", false),
            ("bottom", false),
            ("initial", true),
        ] {
            let key = format!("{}-{phase}-{reduced}", viewport[0]);
            states.insert(key.clone(), json!({"route":"/","viewport":viewport,"reduced_motion":reduced,"phase":phase,"inventory":["hero","navigation"],"tolerances":{"hero.bounds.height":1}}));
            captures.insert(key, json!({"route":"/","viewport":viewport,"reduced_motion":reduced,"inventory":["hero","navigation"],"facts":{"hero.bounds.height":900,"hero.title":"Hello","navigation.expanded":false},"errors":[],"settled":true}));
        }
    }
    (
        json!({"schema_version":1,"reference_sha256":"reference","project_sha256":"project","routes":["/"],"states":states,"gaps":[]}),
        json!({"schema_version":1,"project_sha256":null,"states":captures}),
    )
}
fn run(
    contract: Value,
    reference: Value,
    candidate: Value,
) -> anyhow::Result<(usize, Vec<String>)> {
    compare(
        &serde_json::from_value::<Contract>(contract)?,
        &serde_json::from_value::<Capture>(reference)?,
        &serde_json::from_value::<Capture>(candidate)?,
    )
}
#[test]
fn comparison_rejects_missing_sections_states_and_changed_behavior() {
    let (contract, reference) = fixture();
    assert!(
        run(contract.clone(), reference.clone(), reference.clone())
            .unwrap()
            .1
            .is_empty()
    );
    for (pointer, value) in [
        (
            "/states/390-initial-false/facts/hero.bounds.height",
            json!(1200),
        ),
        (
            "/states/390-initial-false/facts/navigation.expanded",
            json!(true),
        ),
        ("/states/390-initial-false/inventory", json!(["hero"])),
        ("/states/390-initial-false/settled", json!(false)),
        ("/states/390-initial-false/errors", json!(["shader failed"])),
        ("/states/390-initial-false/facts", json!({})),
    ] {
        let mut candidate = reference.clone();
        *candidate.pointer_mut(pointer).unwrap() = value;
        assert!(
            !run(contract.clone(), reference.clone(), candidate)
                .unwrap()
                .1
                .is_empty(),
            "accepted {pointer}"
        );
    }
    let mut candidate = reference.clone();
    candidate["states"]
        .as_object_mut()
        .unwrap()
        .remove("390-bottom-false");
    assert!(!run(contract, reference, candidate).unwrap().1.is_empty());
}
#[test]
fn empty_coverage_gaps_and_relaxed_tolerances_cannot_pass() {
    let (contract, reference) = fixture();
    for (pointer, value) in [
        ("/gaps", json!(["scroll not checked"])),
        ("/routes", json!(["/", "/about"])),
        (
            "/states/390-initial-false/tolerances/hero.bounds.height",
            json!(200),
        ),
        (
            "/states/390-initial-false/tolerances",
            json!({"hero.progress": 1}),
        ),
        ("/states", json!({})),
    ] {
        let mut c = contract.clone();
        *c.pointer_mut(pointer).unwrap() = value;
        assert!(
            run(c, reference.clone(), reference.clone()).is_err(),
            "accepted {pointer}"
        );
    }
}
#[test]
fn source_fingerprint_invalidates_after_changes_and_new_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("src/routes")).unwrap();
    std::fs::write(root.join("package.json"), "{}").unwrap();
    std::fs::write(root.join("src/routes/index.tsx"), "first").unwrap();
    let before = project_fingerprint(root).unwrap();
    std::fs::write(root.join("src/routes/index.tsx"), "second").unwrap();
    let edited = project_fingerprint(root).unwrap();
    assert_ne!(before, edited);
    std::fs::write(root.join("src/routes/about.ts"), "new").unwrap();
    assert_ne!(edited, project_fingerprint(root).unwrap());
    let current = project_fingerprint(root).unwrap();
    std::fs::create_dir(root.join(".output")).unwrap();
    std::fs::write(root.join(".output/build"), "ignored").unwrap();
    assert_eq!(current, project_fingerprint(root).unwrap());
}
#[test]
fn gate_binds_reference_source_and_route_inventory() {
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("app");
    std::fs::create_dir_all(root.join("src/routes")).unwrap();
    std::fs::write(root.join("package.json"), "{}").unwrap();
    std::fs::write(
        root.join("src/routes/index.tsx"),
        "// createFileRoute('/fake') must not create route evidence\nexport const Route = createFileRoute('/')({});",
    )
    .unwrap();
    let (mut contract, reference) = fixture();
    let mut candidate = reference.clone();
    let bytes = serde_json::to_vec(&reference).unwrap();
    contract["reference_sha256"] = format!("{:x}", Sha256::digest(&bytes)).into();
    contract["project_sha256"] = project_fingerprint(&root).unwrap().into();
    candidate["project_sha256"] = contract["project_sha256"].clone();
    let c = temp.path().join("contract.json");
    let r = temp.path().join("reference.json");
    let b = temp.path().join("candidate.json");
    let out = temp.path().join("report.json");
    std::fs::write(&c, serde_json::to_vec(&contract).unwrap()).unwrap();
    std::fs::write(&r, &bytes).unwrap();
    std::fs::write(&b, serde_json::to_vec(&candidate).unwrap()).unwrap();
    mochi::parity::check(&root, &c, &r, &b, &out).unwrap();
    assert!(
        mochi::parity::check(&root, &c, &r, &b, &out)
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    let out = temp.path().join("new-report.json");
    std::fs::write(
        root.join("src/routes/about.ts"),
        "export const Route=createFileRoute('/about/')({});",
    )
    .unwrap();
    assert!(
        mochi::parity::check(&root, &c, &r, &b, &out)
            .unwrap_err()
            .to_string()
            .contains("every current generated route")
    );
    std::fs::remove_file(root.join("src/routes/about.ts")).unwrap();
    std::fs::write(root.join(".env"), "PUBLIC_THEME=blue").unwrap();
    assert!(
        mochi::parity::check(&root, &c, &r, &b, &out)
            .unwrap_err()
            .to_string()
            .contains("stale candidate")
    );
    std::fs::remove_file(root.join(".env")).unwrap();
    std::fs::write(&r, b"{}").unwrap();
    assert!(mochi::parity::check(&root, &c, &r, &b, &out).is_err());
}
