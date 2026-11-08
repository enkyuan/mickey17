#[path = "source_fixture.rs"]
mod source_fixture;
use serde_json::Value;
use std::path::Path;
pub fn fixture(root: &Path) -> Value {
    source_fixture::fixture(root, 1, 1);
    for relative in [
        "reconstruction/pages/0-0.json",
        "reconstruction/manifests/0.json",
    ] {
        let path = root.join(relative);
        let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["url"] = "https://example.test/catalog".into();
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    let model = mochi::project::derive(root).unwrap();
    let mut spec: Value =
        serde_json::from_str(include_str!("../fixtures/authored/ui.json")).unwrap();
    spec["input_sha256"] = model.input_sha256.into();
    for c in spec["components"].as_object_mut().unwrap().values_mut() {
        c["evidence"][0]["observation"] = model.observations.keys().next().unwrap().clone().into();
    }
    spec
}
