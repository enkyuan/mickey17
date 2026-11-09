use super::*;
use std::path::PathBuf;

const SOURCE: &str = "const clamp = (e) => Math.max(0, Math.min(1, e));\nlet z = 0;\nconst lerp = (a, b, t) => a + (b - a) * t;\nfunction ease(e) {\n\tlet t = clamp(e);\n\treturn lerp(0, 1, t * t);\n}\nconst bad = (e) => e + z;\nconst writes = () => { z = 1; };\nconst usesThis = function () { return this; };\n";

fn write(dir: &Path, spec: &str) -> PathBuf {
    std::fs::write(dir.join("source.js"), SOURCE).unwrap();
    let spec = spec.replace("HASH", &fingerprint(SOURCE.as_bytes()));
    let path = dir.join("lift.json");
    std::fs::write(&path, spec).unwrap();
    path
}
fn by_name<'a>(rows: &'a [SurveyRow], name: &str) -> &'a SurveyRow {
    rows.iter().find(|r| r.name == name).unwrap()
}

#[test]
fn survey_separates_closed_functions_from_captured_state() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("source.js"), SOURCE).unwrap();
    let rows = survey(&dir.path().join("source.js"), None).unwrap();
    for name in ["clamp", "lerp", "ease"] {
        assert!(by_name(&rows, name).liftable, "{name}");
    }
    assert_eq!(by_name(&rows, "bad").reasons, ["reads captured `z`"]);
    assert_eq!(by_name(&rows, "writes").reasons, ["writes captured `z`"]);
    assert_eq!(by_name(&rows, "usesThis").reasons, ["uses `this`"]);
}

#[test]
fn lifts_named_typed_functions_with_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let spec = write(
        dir.path(),
        r#"{"schema_version":1,"source":"source.js","source_sha256":"HASH","functions":[
              {"name":"clamp","line":1,"as":"clamp01","params":[{"from":"e","name":"value","type":"number"}],"returns":"number"},
              {"name":"lerp","line":3,"as":"mix","params":[{"from":"a","name":"start","type":"number"},{"from":"b","name":"end","type":"number"},{"from":"t","name":"amount","type":"number"}],"returns":"number"},
              {"name":"ease","line":4,"as":"easeIn","locals":{"t":"clamped"},"params":[{"from":"e","name":"input","type":"number"}],"returns":"number"}]}"#,
    );
    let out = dir.path().join("math.ts");
    let manifest = lift(&spec, &out, &dir.path().join("math.lift.json")).unwrap();
    let module = std::fs::read_to_string(&out).unwrap();
    assert!(module.contains("export function clamp01(value: number): number {\n  return Math.max(0, Math.min(1, value));\n}"), "{module}");
    assert!(module.contains("export function mix(start: number, end: number, amount: number): number {\n  return start + (end - start) * amount;"), "{module}");
    assert!(
        module.contains("let clamped = clamp01(input);\n  return mix(0, 1, clamped * clamped);"),
        "{module}"
    );
    assert_eq!(manifest.functions[2].deps, ["clamp01", "mix"]);
    assert_eq!(manifest.functions[0].original.line, 1);
    assert!(std::fs::metadata(dir.path().join("math.lift.json")).is_ok());
}

#[test]
fn refuses_blocked_stale_and_unnamed_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.ts");
    let manifest = dir.path().join("out.json");
    let blocked = write(
        dir.path(),
        r#"{"schema_version":1,"source":"source.js","source_sha256":"HASH","functions":[
              {"name":"bad","line":8,"as":"bad","params":[{"from":"e","name":"e","type":"number"}],"returns":"number"}]}"#,
    );
    let error = lift(&blocked, &out, &manifest).unwrap_err().to_string();
    assert!(error.contains("reads captured `z`"), "{error}");
    let unnamed = write(
        dir.path(),
        r#"{"schema_version":1,"source":"source.js","source_sha256":"HASH","functions":[
              {"name":"ease","line":4,"as":"easeIn","params":[{"from":"e","name":"input","type":"number"}],"returns":"number"}]}"#,
    );
    let error = lift(&unnamed, &out, &manifest).unwrap_err().to_string();
    assert!(error.contains("must also name"), "{error}");
    let stale = write(
        dir.path(),
        r#"{"schema_version":1,"source":"source.js","source_sha256":"deadbeef","functions":[
              {"name":"clamp","line":1,"as":"clamp01","params":[{"from":"e","name":"value","type":"number"}],"returns":"number"}]}"#,
    );
    let error = lift(&stale, &out, &manifest).unwrap_err().to_string();
    assert!(
        error.contains("does not match the reviewed specification"),
        "{error}"
    );
    assert!(
        !out.exists(),
        "no output may be written when a lift is refused"
    );
}

#[test]
fn rename_collisions_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let merged = write(
        dir.path(),
        r#"{"schema_version":1,"source":"source.js","source_sha256":"HASH","functions":[
              {"name":"lerp","line":3,"as":"mix","params":[{"from":"a","name":"x","type":"number"},{"from":"b","name":"x","type":"number"},{"from":"t","name":"amount","type":"number"}],"returns":"number"}]}"#,
    );
    let error = lift(
        &merged,
        &dir.path().join("o.ts"),
        &dir.path().join("o.json"),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("would merge"), "{error}");
    let shadowing_global = write(
        dir.path(),
        r#"{"schema_version":1,"source":"source.js","source_sha256":"HASH","functions":[
              {"name":"clamp","line":1,"as":"clamp01","params":[{"from":"e","name":"Math","type":"number"}],"returns":"number"}]}"#,
    );
    let error = lift(
        &shadowing_global,
        &dir.path().join("o.ts"),
        &dir.path().join("o.json"),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("collides"), "{error}");
    assert!(!dir.path().join("o.ts").exists());
}

#[test]
fn committed_fixture_is_reproduced_exactly() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("benchmarks/fixtures/lift");
    let out = tempfile::tempdir().unwrap();
    lift(
        &fixture.join("lift.json"),
        &out.path().join("math.ts"),
        &out.path().join("math.lift.json"),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(out.path().join("math.ts")).unwrap(),
        std::fs::read_to_string(fixture.join("math.ts")).unwrap(),
        "regenerate the golden fixture only after reviewing the diff"
    );
    let strip = |path: &Path| {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("tool_version");
        value
    };
    assert_eq!(
        strip(&out.path().join("math.lift.json")),
        strip(&fixture.join("math.lift.json"))
    );
}

#[test]
fn simultaneous_renames_that_swap_names_are_valid() {
    let dir = tempfile::tempdir().unwrap();
    let spec = write(
        dir.path(),
        r#"{"schema_version":1,"source":"source.js","source_sha256":"HASH","functions":[
              {"name":"lerp","line":3,"as":"mix","params":[{"from":"a","name":"b","type":"number"},{"from":"b","name":"a","type":"number"},{"from":"t","name":"amount","type":"number"}],"returns":"number"}]}"#,
    );
    let out = dir.path().join("o.ts");
    lift(&spec, &out, &dir.path().join("o.json")).unwrap();
    assert!(
        std::fs::read_to_string(out)
            .unwrap()
            .contains("return b + (a - b) * amount;")
    );
}

fn context_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("benchmarks/fixtures/lift/context")
}
/// Lifts the context fixture after the source and the specification have been altered.
/// Function lines are recomputed from the altered source. Returns the module text too.
fn lift_full(
    edit: impl Fn(&str) -> String,
    edit_spec: impl Fn(&mut serde_json::Value),
) -> Result<(String, Manifest)> {
    let dir = tempfile::tempdir().unwrap();
    let source = edit(&std::fs::read_to_string(context_fixture().join("source.js")).unwrap());
    std::fs::write(dir.path().join("source.js"), &source).unwrap();
    let mut spec: serde_json::Value =
        serde_json::from_slice(&std::fs::read(context_fixture().join("lift.json")).unwrap())
            .unwrap();
    spec["source_sha256"] = fingerprint(source.as_bytes()).into();
    edit_spec(&mut spec);
    for function in spec["functions"].as_array_mut().unwrap() {
        let name = function["name"].as_str().unwrap().to_owned();
        if let Some(line) = source.lines().position(|l| {
            l.contains(&format!("function {name}("))
                || l.contains(&format!("const {name} ="))
                || l.contains(&format!("var {name} ="))
        }) {
            function["line"] = (line + 1).into();
        }
    }
    std::fs::write(dir.path().join("lift.json"), spec.to_string()).unwrap();
    let manifest = lift(
        &dir.path().join("lift.json"),
        &dir.path().join("o.ts"),
        &dir.path().join("o.json"),
    )?;
    Ok((
        std::fs::read_to_string(dir.path().join("o.ts")).unwrap(),
        manifest,
    ))
}
fn lift_variant(edit: impl Fn(&str) -> String) -> Result<Manifest> {
    lift_full(edit, |_| {}).map(|(_, m)| m)
}
/// The fixture with `table` and `geom` as plain (unfrozen) literals.
fn plain(s: &str) -> String {
    s.replace(
        "Object.freeze([Object.freeze([1, 2]), Object.freeze([3, 4])])",
        "[[1, 2], [3, 4]]",
    )
    .replace(
        "Object.freeze({ config: Object.freeze({ size: 90, label: \"garden\" }) })",
        "{ config: { size: 90, label: \"garden\" } }",
    )
}
/// Lifts the unfrozen fixture with a reviewer's assumption for `table` and `geom`.
fn assumed_spec(spec: &mut serde_json::Value) {
    let layout = spec["functions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|f| f["name"] == "layout")
        .unwrap();
    layout["memo"]["reviewed_assumptions"] = serde_json::json!([
        {"root": "table", "reason": "test"},
        {"root": "geom", "reason": "test"}
    ]);
}
fn refused_assumed(edit: impl Fn(&str) -> String, expected: &str) {
    let error = match lift_full(|s| edit(&plain(s)), assumed_spec) {
        Err(e) => format!("{e:#}"),
        Ok(_) => panic!("the lift was accepted, but `{expected}` was expected"),
    };
    assert!(
        error.contains(expected),
        "expected `{expected}` in: {error}"
    );
}
fn refused(edit: impl Fn(&str) -> String, expected: &str) {
    let error = match lift_variant(edit) {
        Err(e) => format!("{e:#}"),
        Ok(_) => panic!("the lift was accepted, but `{expected}` was expected"),
    };
    assert!(
        error.contains(expected),
        "expected `{expected}` in: {error}"
    );
}

#[test]
fn map_is_allowed_but_unlisted_globals_still_block() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("s.js"),
        "const a = () => new Map([[1, 2]]);\nconst b = () => new Set([1]);\n",
    )
    .unwrap();
    let rows = survey(&dir.path().join("s.js"), None).unwrap();
    assert!(by_name(&rows, "a").liftable);
    assert_eq!(by_name(&rows, "b").reasons, ["global `Set`"]);
}

#[test]
fn committed_context_fixture_is_reproduced_exactly() {
    let fixture = context_fixture();
    let out = tempfile::tempdir().unwrap();
    let manifest = lift(
        &fixture.join("lift.json"),
        &out.path().join("layout.ts"),
        &out.path().join("layout.lift.json"),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(out.path().join("layout.ts")).unwrap(),
        std::fs::read_to_string(fixture.join("layout.ts")).unwrap(),
        "regenerate the golden fixture only after reviewing the diff"
    );
    let strip = |path: &Path| {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("tool_version");
        value
    };
    assert_eq!(
        strip(&out.path().join("layout.lift.json")),
        strip(&fixture.join("layout.lift.json"))
    );
    let rows = manifest
        .functions
        .iter()
        .find(|f| f.export == "rowsFor")
        .unwrap();
    let proof = rows.memo.as_ref().unwrap();
    assert_eq!(proof.key_fields, ["width", "height"]);
    assert!(
        proof
            .cache_writes
            .iter()
            .any(|w| w.kind.starts_with("fill"))
    );
    assert_eq!(proof.coupled[0].cache, "dom");
    assert_eq!(proof.coupled[0].resets_paired_with_cache_reset.len(), 2);
    let unlifted: Vec<_> = manifest.unlifted.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(unlifted, ["sample", "resize", "settle"]);
    assert!(
        manifest.unlifted[0]
            .reasons
            .contains(&"global `document`".to_owned())
    );
}

#[test]
fn reads_and_writes_outside_the_declared_context_are_refused() {
    refused(
        |s| s.replace("geom.config.size * .85", "geom.config.label.length * .85"),
        "reads captured `geom` outside the declared context fields (geom.config.size)",
    );
    refused(
        |s| s.replace("let z = sample(),", "w = 1; let z = sample(),"),
        "writes captured `w`",
    );
    refused(
        |s| {
            s.replace(
                "let z = sample(),",
                "geom.config.size = 3; let z = sample(),",
            )
        },
        "writes through captured `geom`",
    );
    refused(
        |s| s.replace("let z = sample(),", "table.push([1]); let z = sample(),"),
        "writes through captured `table`",
    );
    refused(
        |s| {
            s.replace(
                "let z = sample(),",
                "delete geom.config.size; let z = sample(),",
            )
        },
        "writes through captured `geom`",
    );
    refused(
        |s| s.replace("let k = `${w}x${h}`", "let k = `${w}x${h}${other}`") + "let other = 1;\n",
        "reads captured `other`",
    );
}

#[test]
fn a_cache_that_is_not_provably_neutral_keeps_the_function_blocked() {
    refused(
        |s| s.to_owned() + "function poison() { cache = { key: \"x\", rows: null, mid: 0 }; }\n",
        "assigned something other than null or the memoized value",
    );
    refused(
        |s| s.to_owned() + "function bump() { cache++; }\n",
        "in a way the proof does not model",
    );
    refused(
        |s| s.to_owned() + "function stale() { dom = null; }\n",
        "is reset at line",
    );
    refused(
        |s| s.to_owned() + "function apart() { cache = null; dom = null; }\n",
        "without resetting `cache` in the same expression",
    );
    refused(
        |s| s.to_owned() + "function refill() { dom = { span: 1, top: 0 }; }\n",
        "assigned outside `sample`",
    );
    refused(
        |s| s.replace("`${w}x${h}`", "`${w}x${h}${clamp(w)}`"),
        "key expression is not a pure expression",
    );
    refused(
        |s| s.replace("`${w}x${h}`", "`${w}`"),
        "input `h` is not covered by the key",
    );
    refused(
        |s| s.replace("return layout(), cache.mid;", "return cache.mid;"),
        "body is not `return F(), C.field`",
    );
    refused(
        |s| s.replace("cache.key !== k", "cache.key !== k + 1"),
        "not a write-back memo function",
    );
}

#[test]
fn inputs_outside_the_key_must_be_shown_stable() {
    refused(
        |s| {
            s.replace("const settings", "let settings")
                + "function swap() { settings = { gap: 1, bias: 2 }; }\n"
        },
        "`settings` is reassigned at line",
    );
    refused(
        |s| s.replace("const table", "var table") + "var table = [[5, 6], [7, 8]];\n",
        "`table` is declared more than once",
    );
    // Frozen roots get the same direct checks as reviewed ones: the proof is conditional on
    // an intact `Object.freeze`, so the lifter does not rely on it alone.
    for (extra, expected) in [
        (
            "function leak() { return settings; }\n",
            "`settings` is used as a value",
        ),
        (
            "function tweak() { settings.gap = 1; }\n",
            "`settings` is written through",
        ),
        (
            "function alias() { const g = geom.config; g.size = 3; }\n",
            "`geom.config` is used as a value",
        ),
        (
            "function loop() { table.forEach((r) => { r[0] = 99; }); }\n",
            "callback passed to `table.forEach`",
        ),
    ] {
        refused(|s| s.to_owned() + extra, expected);
    }
    // Not frozen, and nobody vouched for it: nothing proves it stable.
    refused(plain, "not an Object.freeze'd literal at every level");
    refused(
        |s| {
            s.replace(
                "Object.freeze({ gap: 12, bias: .5 })",
                "Object.freeze({ gap: 12, get bias() { return .5; } })",
            )
        },
        "not an Object.freeze'd literal at every level",
    );
    refused(
        |s| {
            s.replace(
                "Object.freeze({ gap: 12, bias: .5 })",
                "{ gap: 12, bias: .5 }",
            )
        },
        "not an Object.freeze'd literal at every level",
    );
    // Shadowing `Object` does not make a literal frozen.
    refused(
        |s| {
            s.replace(
                "const settings = Object.freeze({ gap: 12, bias: .5 });",
                "const Object = { freeze: (o) => ({ gap: o.gap, get bias() { return .5; } }) }, settings = Object.freeze({ gap: 12, bias: .5 });",
            )
        },
        "not an Object.freeze'd literal at every level",
    );
    // With a reviewer's word the direct checks still run.
    refused_assumed(
        |s| s.to_owned() + "function grow() { table.push([1]); }\n",
        "`table` is written through at line",
    );
    refused_assumed(
        |s| s.to_owned() + "function other() { return geom.config.size = 4; }\n",
        "`geom` is written through at line",
    );
    refused_assumed(
        |s| s.to_owned() + "function leak() { return table; }\n",
        "`table` is used as a bare value",
    );
    refused_assumed(
        |s| s.to_owned() + "function alias() { let c = geom.config; c.size = 200; }\n",
        "`geom.config` is used as a value",
    );
    refused_assumed(
        |s| s.to_owned() + "function loop() { table.forEach((r) => { r[0] = 99; }); }\n",
        "callback passed to `table.forEach`",
    );
}

#[test]
fn aliasing_nondeterminism_and_non_empty_initial_caches_are_refused() {
    refused(
        |s| {
            s.replace(
                "let z = sample(),",
                "let q = table[0]; q[0] = 9; let z = sample(),",
            )
        },
        "may alias a captured input",
    );
    refused(
        |s| {
            s.replace(
                "(edge - z.top) * .98",
                "(edge - z.top) * .98 + Math.random() * 0",
            )
        },
        "`Math.random`, which is not on the pure allowlist",
    );
    refused(
        |s| {
            s.replace(
                "cache = null, dom = null;\nfunction sample",
                "cache = { key: \"x\", rows: new Map(), mid: 0 }, dom = null;\nfunction sample",
            )
        },
        "not declared empty",
    );
    refused(
        |s| {
            s.replace(
                "dom = null;\nfunction sample",
                "dom = { span: 1, top: 1 };\nfunction sample",
            )
        },
        "it mentions `dom`",
    );
}

/// Lifts `source` with the given function specs (JSON array) and spec-level extras.
fn lift_inline(
    source: &str,
    functions: serde_json::Value,
    extra: serde_json::Value,
) -> Result<(String, Manifest)> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("s.js"), source).unwrap();
    let mut spec = serde_json::json!({
        "schema_version": 1,
        "source": "s.js",
        "source_sha256": fingerprint(source.as_bytes()),
        "functions": functions,
    });
    for (k, v) in extra.as_object().into_iter().flatten() {
        spec[k] = v.clone();
    }
    std::fs::write(dir.path().join("l.json"), spec.to_string()).unwrap();
    let manifest = lift(
        &dir.path().join("l.json"),
        &dir.path().join("o.ts"),
        &dir.path().join("o.json"),
    )?;
    Ok((
        std::fs::read_to_string(dir.path().join("o.ts")).unwrap(),
        manifest,
    ))
}
fn one(name: &str, line: usize, export: &str, params: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "name": name, "line": line, "as": export,
        "params": params.iter().map(|p| serde_json::json!({"from": p, "name": p, "type": "number"})).collect::<Vec<_>>(),
        "returns": "number"
    })
}
fn inline_error(source: &str, functions: serde_json::Value, extra: serde_json::Value) -> String {
    format!("{:#}", lift_inline(source, functions, extra).unwrap_err())
}

#[test]
fn writes_to_outer_bindings_and_unstable_dependencies_are_blocked() {
    let reassigned = "let f = (e) => e * 2;\nconst g = (e) => f(e) + 1;\nf = (e) => e * 3;\n";
    let error = inline_error(
        reassigned,
        serde_json::json!([one("f", 1, "f", &["e"]), one("g", 2, "g", &["e"])]),
        serde_json::json!({}),
    );
    assert!(
        error.contains("depends on `f`, which is reassigned"),
        "{error}"
    );
    let error = inline_error(
        "const f = (e) => { f = null; return e; };\n",
        serde_json::json!([one("f", 1, "f", &["e"])]),
        serde_json::json!({}),
    );
    assert!(error.contains("writes captured `f`"), "{error}");
    let error = inline_error(
        "const f = (e) => e;\nconst g = () => { f = 1; };\n",
        serde_json::json!([one("g", 2, "g", &[])]),
        serde_json::json!({}),
    );
    assert!(error.contains("writes captured `f`"), "{error}");
    let error = inline_error(
        "var f = (e) => e;\nvar f = (e) => e + 1;\nconst g = (e) => f(e);\n",
        serde_json::json!([one("g", 3, "g", &["e"])]),
        serde_json::json!({}),
    );
    assert!(error.contains("declared more than once"), "{error}");
    let error = inline_error(
        "let f = (e) => f(e - 1);\nf = null;\n",
        serde_json::json!([one("f", 1, "f", &["e"])]),
        serde_json::json!({}),
    );
    assert!(error.contains("which is reassigned"), "{error}");
}

#[test]
fn shorthand_in_assignment_patterns_blocks_renames() {
    for source in [
        "const f = (p) => { let x; ({ x } = p[0]); return x; };\n",
        "const f = (p) => { let x; ({ x = 1 } = p[0]); return x; };\n",
    ] {
        let mut function = one("f", 1, "f", &["p"]);
        function["locals"] = serde_json::json!({"x": "y"});
        let error = inline_error(source, serde_json::json!([function]), serde_json::json!({}));
        assert!(error.contains("shorthand"), "{error}");
    }
}

#[test]
fn line_breaks_inside_literals_are_preserved_exactly() {
    let source = "function f(e) {\n\tconst s = `a\n\t\tb ${e}\n\t c`;\n\tconst t = 'x\\\n\t\ty';\n\treturn s.length + t.length + e;\n}\n";
    let (module, _) = lift_inline(
        source,
        serde_json::json!([one("f", 1, "f", &["e"])]),
        serde_json::json!({}),
    )
    .unwrap();
    assert!(module.contains("`a\n\t\tb ${e}\n\t c`"), "{module}");
    assert!(module.contains("'x\\\n\t\ty'"), "{module}");
    assert!(module.contains("  const s = `a"), "{module}");
}

#[test]
fn named_function_expressions_keep_their_self_reference() {
    let (module, _) = lift_inline(
        "const f = function g(n) { return n > 0 ? g(n - 1) + 1 : 0; };\n",
        serde_json::json!([one("f", 1, "countDown", &["n"])]),
        serde_json::json!({}),
    )
    .unwrap();
    assert!(module.contains("countDown(n - 1) + 1"), "{module}");
    assert!(!module.contains("g("), "{module}");
}

#[test]
fn export_names_cannot_shadow_globals_or_reserved_words() {
    let error = inline_error(
        "const f = (e) => Math.abs(e);\n",
        serde_json::json!([one("f", 1, "Math", &["e"])]),
        serde_json::json!({}),
    );
    assert!(error.contains("collides"), "{error}");
    for word in [
        "interface",
        "implements",
        "package",
        "private",
        "protected",
        "public",
    ] {
        let error = inline_error(
            "const f = (e) => e;\n",
            serde_json::json!([one("f", 1, word, &["e"])]),
            serde_json::json!({}),
        );
        assert!(
            error.contains("not a usable export name"),
            "{word}: {error}"
        );
    }
    let error = inline_error(
        "const f = (e) => e;\n",
        serde_json::json!([one("f", 1, "Point2", &["e"])]),
        serde_json::json!({"imports": ["import type { Point2 } from './t';"]}),
    );
    assert!(error.contains("collides"), "{error}");
}

#[test]
fn meta_properties_super_and_reflection_block_the_lift() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
            dir.path().join("s.js"),
            "const a = () => import.meta.url;\nconst b = function () { return new.target; };\nclass K extends Object { m() { const c = () => super.toString(); return c; } }\nconst d = (e) => e.constructor.constructor('return 1')();\nconst h = (e) => e['__proto__'];\nconst i = (e) => e.prototype;\n",
        )
        .unwrap();
    let rows = survey(&dir.path().join("s.js"), None).unwrap();
    assert!(
        by_name(&rows, "a")
            .reasons
            .iter()
            .any(|r| r.contains("import.meta"))
    );
    assert!(
        by_name(&rows, "b")
            .reasons
            .iter()
            .any(|r| r.contains("new.target"))
    );
    assert!(
        by_name(&rows, "c")
            .reasons
            .iter()
            .any(|r| r.contains("super"))
    );
    for name in ["d", "h", "i"] {
        assert!(
            by_name(&rows, name)
                .reasons
                .iter()
                .any(|r| r.contains("reaches")),
            "{name}"
        );
    }
}

#[test]
fn global_writes_and_nondeterminism_block_the_lift() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
            dir.path().join("s.js"),
            "const a = (e) => { Math = null; return e; };\nconst b = (e) => { Math.__memo = e; return e; };\nconst c = (e) => Math.random() + e;\nconst d = (e) => { const M = Math; return M.max(e, 1); };\nconst ok = (e) => Math.max(e, 1);\nconst f = (e) => { Array.prototype.x = e; return e; };\nconst g = (e) => { Number.cache = e; return e; };\n",
        )
        .unwrap();
    let rows = survey(&dir.path().join("s.js"), None).unwrap();
    let has = |n: &str, what: &str| by_name(&rows, n).reasons.iter().any(|r| r.contains(what));
    assert!(has("a", "writes global `Math`"));
    assert!(has("b", "writes through global `Math`"));
    assert!(has("c", "Math.random"));
    assert!(has("d", "Math"));
    assert!(by_name(&rows, "ok").liftable);
    assert!(has("f", "writes through global `Array`") || has("f", "reaches"));
    assert!(has("g", "writes through global `Number`"));
}

#[test]
fn typescript_that_would_parse_javascript_differently_is_refused() {
    let error = inline_error(
        "const f = (a, b, c) => a<b>(c + 1);\n",
        serde_json::json!([serde_json::json!({
                "name": "f", "line": 1, "as": "f",
                "params": [{"from":"a","name":"a","type":"number"},{"from":"b","name":"b","type":"number"},{"from":"c","name":"c","type":"number"}],
                "returns": "number"})]),
        serde_json::json!({}),
    );
    assert!(
        error.contains("reads the original code differently"),
        "{error}"
    );
}

#[test]
fn specification_text_cannot_inject_code() {
    let mut documented = one("f", 1, "f", &["e"]);
    documented["doc"] = "x */ export const evil = 1; /*".into();
    let error = inline_error(
        "const f = (e) => e;\n",
        serde_json::json!([documented]),
        serde_json::json!({}),
    );
    assert!(error.contains("*/"), "{error}");
    for bad in [
        "number { return 1; } export const evil = 1; function g(): number",
        "number // {",
        "number /* x",
        "number extends",
    ] {
        let mut function = one("f", 1, "f", &["e"]);
        function["returns"] = bad.into();
        let error = inline_error(
            "const f = (e) => e;\n",
            serde_json::json!([function]),
            serde_json::json!({}),
        );
        assert!(error.contains("empty or unsafe"), "{bad}: {error}");
    }
    let mut param = one("f", 1, "f", &["e"]);
    param["params"][0]["type"] = "number) { evil(); } function g(x: number".into();
    let error = inline_error(
        "const f = (e) => e;\n",
        serde_json::json!([param]),
        serde_json::json!({}),
    );
    assert!(error.contains("empty or unsafe"), "{error}");
    for bad in [
        "import type { A } from './a'; export const evil = 1;",
        "import { A } from './a';",
        "import type { A } from './a'; // x",
    ] {
        let error = inline_error(
            "const f = (e) => e;\n",
            serde_json::json!([one("f", 1, "f", &["e"])]),
            serde_json::json!({"imports": [bad]}),
        );
        assert!(error.contains("imports must be"), "{bad}: {error}");
    }
    let error = inline_error(
        "const f = (e) => e;\n",
        serde_json::json!([one("f", 1, "f", &["e"])]),
        serde_json::json!({"declarations": ["export type A = number; export const evil = 1;"]}),
    );
    assert!(error.contains("declarations must be"), "{error}");
}

#[test]
fn a_failed_manifest_write_leaves_no_module_behind() {
    let dir = tempfile::tempdir().unwrap();
    let source = "const f = (e) => e;\n";
    std::fs::write(dir.path().join("s.js"), source).unwrap();
    let spec = serde_json::json!({"schema_version": 1, "source": "s.js", "source_sha256": fingerprint(source.as_bytes()), "functions": [one("f", 1, "f", &["e"])]});
    std::fs::write(dir.path().join("l.json"), spec.to_string()).unwrap();
    let module = dir.path().join("o.ts");
    let error = lift(
        &dir.path().join("l.json"),
        &module,
        &dir.path().join("missing/o.json"),
    )
    .unwrap_err();
    assert!(!module.exists(), "{error:#}");
    assert!(
        std::fs::read_dir(dir.path()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp"))
    );
    // An existing manifest also leaves nothing new behind.
    std::fs::write(dir.path().join("o.json"), "x").unwrap();
    assert!(
        lift(
            &dir.path().join("l.json"),
            &module,
            &dir.path().join("o.json")
        )
        .is_err()
    );
    assert!(!module.exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("o.json")).unwrap(),
        "x"
    );
}

#[test]
fn local_ordinals_count_distinct_bindings_not_redeclarations() {
    let mut function = one("f", 1, "f", &["a"]);
    function["locals"] = serde_json::json!({"t#2": "inner"});
    let (module, _) = lift_inline(
        "function f(a) { var t = 1; var t = 2; { let t = 3; a += t; } return t + a; }\n",
        serde_json::json!([function]),
        serde_json::json!({}),
    )
    .unwrap();
    assert!(module.contains("let inner = 3; a += inner;"), "{module}");
    assert!(module.contains("return t + a;"), "{module}");
}

#[test]
fn committed_hazard_fixture_is_reproduced_exactly() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("benchmarks/fixtures/lift/hazards");
    let out = tempfile::tempdir().unwrap();
    lift(
        &fixture.join("lift.json"),
        &out.path().join("hazards.ts"),
        &out.path().join("hazards.lift.json"),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(out.path().join("hazards.ts")).unwrap(),
        std::fs::read_to_string(fixture.join("hazards.ts")).unwrap(),
        "regenerate the golden fixture only after reviewing the diff"
    );
    let strip = |path: &Path| {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("tool_version");
        value
    };
    assert_eq!(
        strip(&out.path().join("hazards.lift.json")),
        strip(&fixture.join("hazards.lift.json"))
    );
}

#[test]
fn review_findings_on_caches_samplers_keys_and_aliases_stay_blocked() {
    refused(
        |s| s.to_owned() + "function poke() { if (cache) cache.mid = 99; }\n",
        "cache `cache` is edited in place",
    );
    refused(
        |s| s.to_owned() + "function nudge() { dom && (dom.top = 5); }\n",
        "`dom` is edited in place",
    );
    refused(
        |s| s.replace("if (!dom) {", "{"),
        "not a guarded cache getter",
    );
    refused(
        |s| s.to_owned() + "function settle2() { cache = null, layout(), dom = null; }\n",
        "without resetting `cache` in the same expression",
    );
    refused_assumed(
        |s| s.to_owned() + "function grow() { let c = geom.config; c.size = 200; }\n",
        "`geom.config` is used as a value",
    );
    refused(
        |s| s.replace("`${w}x${h}`", "w + h"),
        "not a template literal of numeric declared fields",
    );
    refused(
        |s| s.replace("`${w}x${h}`", "`${w}${h}`"),
        "can occur inside a printed number",
    );
    refused(
        |s| s.replace("`${w}x${h}`", "`${w}x${h}x${table.length}`"),
        "other than a bare numeric declared field",
    );
    refused(
        |s| {
            s.replace(
                "if (!cache || cache.key !== k) {",
                "if (!cache || cache.key !== k) { if (w > 5000) return null;",
            )
        },
        "contains a `return`",
    );
    refused(
        |s| {
            s.replace(
                "return cache.rows;",
                "return cache.key === k ? cache.rows : null;",
            )
        },
        "not exactly `return C.field;`",
    );
    refused(
        |s| {
            s.replace(
                "let z = sample(),",
                "let q = { t: table }; q.t.push(1); let z = sample(),",
            )
        },
        "may alias a captured input",
    );
    refused(
        |s| s.replace("let z = sample(),", "table[\"push\"](1); let z = sample(),"),
        "writes through captured `table`",
    );
    refused(
        |s| {
            s.replace("let z = sample(),", "bump(table); let z = sample(),")
                + "function bump(a) { a.push(1); }\n"
        },
        "used as a bare value",
    );
    refused(
        |s| s.replace("left: left };", "left: left }; h = 1;"),
        "writes the declared input `h`",
    );
}

fn spec_function(name: &str, export: &str, params: &[&str], context: bool) -> serde_json::Value {
    let mut f = one(name, 1, export, params);
    if context {
        f["context"] = serde_json::json!({"type": "Inputs", "param": "inputs"});
    }
    f
}
fn push_function(spec: &mut serde_json::Value, f: serde_json::Value) {
    spec["functions"].as_array_mut().unwrap().push(f);
}

#[test]
fn second_review_unsound_lifts_are_refused() {
    // 1. a field name that resolves to a different binding in another function.
    let error = format!(
        "{:#}",
        lift_full(
            |s| s.to_owned() + "{ let w = 7; var inner = (e) => e + w; }\n",
            |spec| push_function(spec, spec_function("inner", "inner", &["e"], true)),
        )
        .unwrap_err()
    );
    assert!(error.contains("different bindings"), "{error}");
    // 2. element writes through calls and callbacks, with the root accepted on a reviewer's word.
    refused(
        |s| {
            s.replace(
                "let z = sample(),",
                "table.at(0)[0] += 1; let z = sample(),",
            )
        },
        "writes through captured `table`",
    );
    refused_assumed(
        |s| s.to_owned() + "function p1() { table.at(0)[0] = 99; }\n",
        "`table` is written through",
    );
    refused_assumed(
        |s| s.to_owned() + "function p2() { table.forEach((r) => { r[0] = 99; }); }\n",
        "callback passed to `table.forEach`",
    );
    refused(
        |s| {
            s.replace(
            "const geom = Object.freeze({ config: Object.freeze({ size: 90, label: \"garden\" }) });",
            "const geom = { config: { size: 90, label: \"garden\" }, grow() { this.config.size += 1; } };",
        ) + "geom.grow();\n"
        },
        "not an Object.freeze'd literal",
    );
    // A helper chain two levels deep that edits the declared data it is given.
    let error = format!(
        "{:#}",
        lift_full(
            |s| s.replace("let z = sample(),", "via(table[0]); let z = sample(),")
                + "function via(r) { return wr(r); }\nfunction wr(r) { r[0] = 99; }\n",
            |spec| {
                push_function(spec, spec_function("via", "via", &["r"], false));
                push_function(spec, spec_function("wr", "wr", &["r"], false));
            },
        )
        .unwrap_err()
    );
    assert!(
        error.contains("writes through a value that may be a parameter"),
        "{error}"
    );
    // 3. the cache aliased and edited elsewhere.
    refused(
        |s| s.to_owned() + "function poke() { const c = cache; if (c) c.mid = 99; }\n",
        "other than as a reset",
    );
    // 4. the key variable reassigned.
    refused(
        |s| {
            s.replace(
                "let s = scaleFor(),",
                "k = `${w + 1}x${h}`; let s = scaleFor(),",
            )
        },
        "key variable is reassigned",
    );
    // 5. a sampler with an early return.
    refused(
        |s| {
            s.replace(
                "let r = document.body",
                "if (w > 99999) return null; let r = document.body",
            )
        },
        "not a guarded cache getter",
    );
    // 6. a memoizing function with parameters.
    let error = format!(
        "{:#}",
        lift_full(
            |s| s.replace("function layout() {", "function layout(q) {"),
            |spec| {
                let f = spec["functions"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|f| f["name"] == "layout")
                    .unwrap();
                f["params"] = serde_json::json!([{"from": "q", "name": "q", "type": "number"}]);
            },
        )
        .unwrap_err()
    );
    assert!(
        error.contains("has parameters") || error.contains("context cannot be appended safely"),
        "{error}"
    );
    // 8. declared data handed to a helper that edits its parameters, and aliases stored in a fresh local.
    let error = format!(
        "{:#}",
        lift_full(
            |s| s.replace("let z = sample(),", "bump(table[0]); let z = sample(),")
                + "function bump(a) { a.push(0); }\n",
            |spec| push_function(spec, spec_function("bump", "bump", &["a"], false)),
        )
        .unwrap_err()
    );
    assert!(
        error.contains("writes through a value that may be a parameter"),
        "{error}"
    );
    refused(
        |s| {
            s.replace(
                "let z = sample(),",
                "let q = { a: [] }; q.a = table[0]; q.a.push(1); let z = sample(),",
            )
        },
        "may alias a captured input",
    );
    // 9. source that contains the lifter's private marker.
    let error = inline_error(
        "const f = (e) => e + '\u{e000}'.length;\n",
        serde_json::json!([one("f", 1, "f", &["e"])]),
        serde_json::json!({}),
    );
    assert!(error.contains("U+E000"), "{error}");
    // 10. static state on allowlisted globals, read or written.
    refused(
        |s| "Number.__k = 3;\n".to_owned() + s,
        "writes to the standard global `Number`",
    );
    refused(
        |s| s.to_owned() + "Math.max = (a, b) => a;\n",
        "writes to the standard global `Math`",
    );
    refused(
        |s| {
            s.replace(
                "(edge - z.top) * .98",
                "(edge - z.top) * .98 + (Number.__k ?? 0)",
            )
        },
        "not on the pure allowlist",
    );
}

#[test]
fn second_review_emission_defects_are_fixed() {
    // 7. a self-recursive context function passes the context to itself.
    let (module, _) = lift_full(
        |s| s.to_owned() + "function deep(n) { return n > 0 ? deep(n - 1) + w : w; }\n",
        |spec| push_function(spec, spec_function("deep", "deep", &["n"], true)),
    )
    .unwrap();
    assert!(module.contains("deep(n - 1, inputs)"), "{module}");
    // 14. a memo function written on one line.
    let (module, _) = lift_full(
        |s| {
            let a = s.find("function layout() {").unwrap();
            let b = s.find("function mid()").unwrap();
            format!(
                "{}{}\n{}",
                &s[..a],
                s[a..b]
                    .replace("\n\t\t", " ")
                    .replace("\n\t", " ")
                    .replace('\n', " "),
                &s[b..]
            )
        },
        |_| {},
    )
    .unwrap();
    assert!(module.contains("export function computeLayout"), "{module}");
}

#[test]
fn large_sources_survey_quickly() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = String::new();
    for i in 0..6000 {
        source.push_str(&format!(
            "const f{i} = (e) => e + {i};\nconst g{i} = (e) => f{i}(e) * 2 + Math.max(e, {i});\n"
        ));
    }
    std::fs::write(dir.path().join("big.js"), &source).unwrap();
    let started = std::time::Instant::now();
    let rows = survey(&dir.path().join("big.js"), None).unwrap();
    assert_eq!(rows.len(), 12000);
    assert!(rows.iter().all(|r| r.liftable));
    assert!(started.elapsed().as_secs() < 20, "{:?}", started.elapsed());
}

#[test]
fn third_review_unsound_lifts_are_refused() {
    // 1. a reader that hands the cached object to a helper chain which edits it.
    let error = format!(
        "{:#}",
        lift_full(
            |s| s.replace("return layout(), cache.mid;", "return layout(), via2(cache.rows), cache.mid;")
                + "function wr2(m) { m && m.set && m.set(1, [0, 0]); return 0; }\nfunction via2(m) { return wr2(m); }\n",
            |spec| {
                push_function(spec, spec_function("wr2", "wr2", &["m"], false));
                push_function(spec, spec_function("via2", "via2", &["m"], false));
            },
        )
        .unwrap_err()
    );
    assert!(
        error.contains("writes through a value that may be a parameter"),
        "{error}"
    );
    // 4. a sampler fill that can be falsy re-samples on every call.
    refused(
        |s| {
            s.replace(
                "dom = { span: span, top: top, left: left };",
                "dom = r.height > 0 && { span: span, top: top, left: left };",
            )
        },
        "is not filled by an object or array literal",
    );
    // 5. a sampler that reaches a key field through a helper.
    refused(
        |s| {
            s.replace(
                "let r = document.body.getBoundingClientRect(), span = r.height / 10, top = r.top + 0, left = r.left + 0;",
                "let r = document.body.getBoundingClientRect(), span = r.height / 10, top = r.top + 0, left = r.left + 0; bump();",
            ) + "function bump() { w = 999; }\n"
        },
        "calls a function defined in this module",
    );
    // 7. a redeclared variable is not one stable binding.
    refused(
        |s| s.replace("const table", "var table") + "var table = [[5, 6], [7, 8]];\n",
        "declared more than once",
    );
    // 9. aliases stored into a fresh local by destructuring, for-of or redeclaration.
    for alias in [
        "let q = { a: [] }; [q.a] = [table[0]]; q.a.push(9);",
        "let q = { a: [] }; for (q.a of [table[0]]); q.a.push(9);",
        "var q = []; var q = table[0]; q.push(9);",
    ] {
        refused(
            |s| s.replace("let z = sample(),", &format!("{alias} let z = sample(),")),
            "may alias a captured input",
        );
    }
    // 8. a context call with the wrong number of arguments cannot take the context last.
    for call in ["deep()", "deep(1, 2)", "deep(...[1])"] {
        let error = format!(
            "{:#}",
            lift_full(
                |s| s.to_owned()
                    + &format!("function deep(n) {{ return n > 0 ? {call} + w : w; }}\n"),
                |spec| push_function(spec, spec_function("deep", "deep", &["n"], true)),
            )
            .unwrap_err()
        );
        assert!(
            error.contains("context cannot be appended safely"),
            "{call}: {error}"
        );
    }
    // Low: introduced names that collide with each other.
    let error = format!(
        "{:#}",
        lift_full(
            |s| s.to_owned(),
            |spec| {
                let layout = spec["functions"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|f| f["name"] == "layout")
                    .unwrap();
                layout["memo"]["result_local"] = "inputs".into();
            },
        )
        .unwrap_err()
    );
    assert!(error.contains("must be different names"), "{error}");
}

#[test]
fn round_three_proof_breakers_are_refused() {
    // 1. code the static proofs cannot see through.
    refused(
        |s| s.to_owned() + "function e() { return eval('1'); }\n",
        "uses the global `eval`",
    );
    refused(
        |s| s.to_owned() + "function f2() { return new Function('return 1'); }\n",
        "uses the global `Function`",
    );
    // 2. exported caches and roots.
    refused(
        |s| s.to_owned() + "export { cache };\n",
        "cache `cache` is exported",
    );
    refused(|s| s.to_owned() + "export { dom };\n", "`dom` is exported");
    refused(
        |s| s.to_owned() + "export { settings };\n",
        "is exported from the module",
    );
    refused(
        |s| s.to_owned() + "export default [cache];\n",
        "cache `cache` is exported",
    );
    refused(
        |s| {
            s.replace(
                "let w = 0, h = 0, cache = null, dom = null;",
                "export let cache = null, dom = null; let w = 0, h = 0;",
            )
        },
        "is exported from the module",
    );
    // 3. a declared path that is not an own property of the frozen literal.
    refused(
        |s| {
            s.replace(
                "Object.freeze({ gap: 12, bias: .5 })",
                "Object.freeze({ gap: 12 })",
            )
        },
        "not an own data property of the frozen literal",
    );
    refused(
        |s| {
            s.replace(
                "Object.freeze({ gap: 12, bias: .5 })",
                "Object.freeze({ gap: 12, bias: .5, __proto__: { x: 1 } })",
            )
        },
        "not an Object.freeze'd literal at every level",
    );
    refused(
        |s| s.to_owned() + "({}).__proto__.bias = 3;\n",
        "`__proto__`",
    );
    refused(
        |s| s.to_owned() + "Reflect.getPrototypeOf(settings).bias = 3;\n",
        "uses `Reflect`",
    );
    // 4. a patched Object.freeze.
    refused(
        |s| s.to_owned() + "Object.defineProperty(Object, \"freeze\", { value: (o) => o });\n",
        "passes `Object` around as a value",
    );
    refused(
        |s| s.to_owned() + "const F = Object.freeze;\n",
        "other than as the callee",
    );
    refused(
        |s| s.to_owned() + "globalThis.Object.freeze = (o) => o;\n",
        "which can change what the static proofs assume",
    );
    // 5. the sampled object edited through the sampler's result, or read through an accessor.
    refused(
        |s| s.to_owned() + "function za() { let z = sample(); z.top = 5; }\n",
        "edited in place",
    );
    refused(
        |s| s.to_owned() + "function zb() { sample().top = 5; }\n",
        "edited in place",
    );
    refused(
        |s| {
            s.replace(
                "dom = { span: span, top: top, left: left };",
                "dom = { get span() { return document.body.clientWidth; }, top: top, left: left };",
            )
        },
        "is not filled by an object or array literal",
    );
    // 6. `var` lets a repeated declaration reset a cache unseen.
    refused(
        |s| {
            s.replace(
                "let w = 0, h = 0, cache = null, dom = null;",
                "var w = 0, h = 0, cache = null, dom = null;",
            )
        },
        "declared with `var`",
    );
    // 7. mutation through call arguments of a reviewed root.
    refused_assumed(
        |s| s.to_owned() + "function a1() { Object.assign(table[0], [9]); }\n",
        "passed on, assigned, returned or stored",
    );
    refused_assumed(
        |s| s.to_owned() + "function a2() { Array.prototype.splice.call(table[0], 0, 1); }\n",
        "passed on, assigned, returned or stored",
    );
    refused_assumed(
        |s| s.to_owned() + "function a3() { Object.defineProperty(table[1], 0, { value: 1 }); }\n",
        "passed on, assigned, returned or stored",
    );
    refused_assumed(
        |s| s.to_owned() + "function a4() { const t = table[0]; return t; }\n",
        "passed on, assigned, returned or stored",
    );
    // A reviewed assumption is tied to the methods the reviewer saw.
    refused_assumed(
        |s| s.to_owned() + "function pk() { return geom.peek(); }\n",
        "differ from the ones the reviewed assumption lists",
    );
}

#[test]
fn round_four_proof_breakers_are_refused() {
    // 1. separators written as escapes or continuations.
    refused(
        |s| s.replace("`${w}x${h}`", "`${w}\\x31${h}`"),
        "can occur inside a printed number",
    );
    refused(
        |s| s.replace("`${w}x${h}`", "`${w}\\\n${h}`"),
        "can occur inside a printed number",
    );
    // 2/3. the sampled object edited or leaked, refilled later, or backed by shared data.
    refused(
        |s| s.to_owned() + "function n1() { let z; z = sample(); z.span = 0.5; }\n",
        "edited in place",
    );
    refused(
        |s| s.to_owned() + "function n2() { const z = (0, sample()); z.span = 0.5; }\n",
        "edited in place",
    );
    refused(
        |s| s.to_owned() + "function n3() { Object.assign(sample(), { span: 0.5 }); }\n",
        "passed on, returned or stored",
    );
    refused(
        |s| {
            s.replace(
                "dom = { span: span, top: top, left: left };",
                "dom = { span: span, top: top, left: left }; document.body.__zone = dom;",
            )
        },
        "other than by the guard, the fill and the final return",
    );
    refused(
        |s| s.replace("dom = { span", "document.body.__zone = dom = { span"),
        "other than by the guard, the fill and the final return",
    );
    refused(
        |s| {
            s.replace(
            "dom = { span: span, top: top, left: left };",
            "dom = { span: span, top: top, left: left }; setTimeout(() => { dom = { span: 0.5, top: top, left: left }; });",
        )
        },
        "a deferred refill",
    );
    refused(
        |s| {
            s.replace(
                "dom = { span: span, top: top, left: left };",
                "dom = { span: span, top: top, left: left, box: BOX };",
            ) + "const BOX = { span: 80 };\n"
        },
        "it mentions `",
    );
    // 4. a tagged template calls the method too.
    refused_assumed(
        |s| s.to_owned() + "function flip() { table.reverse``; }\n",
        "`table` is written through",
    );
    refused(
        |s| s.replace("let z = sample(),", "table.reverse``; let z = sample(),"),
        "writes through captured `table`",
    );
    // 5. patching Object.freeze through computed access or the global object.
    refused(
        |s| "globalThis[\"Object\"][\"freeze\"] = (o) => o;\n".to_owned() + s,
        "which can change what the static proofs assume",
    );
    refused(
        |s| "window[\"Object\"][\"freeze\"] = (o) => o;\n".to_owned() + s,
        "through a computed member",
    );
    refused(
        |s| s.to_owned() + "Object.defineProperty(globalThis, \"Object\", { value: {} });\n",
        "which can change what the static proofs assume",
    );
    // 6. the sampler reaching a module function indirectly.
    refused(
        |s| {
            s.replace(
                "let r = document.body.getBoundingClientRect(), span = r.height / 10, top = r.top + 0, left = r.left + 0;",
                "let r = document.body.getBoundingClientRect(), span = r.height / 10, top = r.top + 0, left = r.left + 0; bump.call();",
            ) + "function bump() { w += 100; }\n"
        },
        "it mentions `bump`",
    );
    refused(
        |s| {
            s.replace(
                "let r = document.body.getBoundingClientRect(), span = r.height / 10, top = r.top + 0, left = r.left + 0;",
                "let r = document.body.getBoundingClientRect(), span = r.height / 10, top = r.top + 0, left = r.left + 0; (0, bump)();",
            ) + "function bump() { w += 100; }\n"
        },
        "it mentions `bump`",
    );
    // 7. hazards reached through computed keys.
    refused(
        |s| s.to_owned() + "function h1() { return globalThis[\"Reflect\"]; }\n",
        "which can change what the static proofs assume",
    );
    refused(
        |s| s.to_owned() + "function h2() { return ({})[\"__proto__\"]; }\n",
        "through a computed member",
    );
    refused(
        |s| s.to_owned() + "function h3() { return (() => 0)[\"constructor\"](\"return 1\")(); }\n",
        "through a computed member",
    );
    // 8. a wrapped callback, and an iterator that hands out elements.
    refused_assumed(
        |s| s.to_owned() + "function w1() { table.forEach((0, (r) => r.reverse())); }\n",
        "callback passed to `table.forEach`",
    );
    refused_assumed(
        |s| s.to_owned() + "function w2() { for (const r of table.values()) r.reverse(); }\n",
        "differ from the ones the reviewed assumption lists",
    );
    // 10. a parenthesized fill is not the memo shape.
    refused(
        |s| {
            s.replace("cache = {", "(cache = {")
                .replace("settings.gap\n\t\t};", "settings.gap\n\t\t});")
        },
        "not a write-back memo function",
    );
}

#[test]
fn memo_read_callers_are_listed() {
    let (_, manifest) = lift_full(
        |s| s.to_owned() + "function poke() { mid().set(1, []); }\n",
        |_| {},
    )
    .unwrap();
    let rows = manifest
        .functions
        .iter()
        .find(|f| f.export == "rowsFor")
        .unwrap();
    let callers = &rows.memo.as_ref().unwrap().callers;
    assert!(callers.contains(&40), "{callers:?}");
}

#[test]
fn round_five_proof_breakers_are_refused() {
    // 1. module code reachable from the sampler by any route.
    let anchor = "let r = document.body.getBoundingClientRect(), span = r.height / 10, top = r.top + 0, left = r.left + 0;";
    let defs = "function bump() { w += 100; }\nconst util = { bump() { w += 100; }, get x() { w += 100; return 1; } };\nclass K { static bump() { w += 100; } }\n";
    for stmt in [
        "new bump();",
        "bump``;",
        "[0].forEach(bump);",
        "[bump][0]();",
        "({ f: bump }).f();",
        "util.bump();",
        "util.x;",
        "K.bump();",
        "for (const q of util) {}",
    ] {
        refused(
            |s| s.replace(anchor, &format!("{anchor} {stmt}")) + defs,
            "it mentions",
        );
    }
    // 2. a fill value that is a local alias of shared data.
    refused(
        |s| {
            s.replace(anchor, &format!("{anchor} let q = shared;"))
                .replace("dom = { span: span,", "dom = { span: q, size: span,")
                + "const shared = document.body.children;\n"
        },
        "not a number computed in the sampler",
    );
    // 3. a declaration in the guarded block that shadows the key.
    refused(
        |s| {
            s.replace(
                "let s = scaleFor(),",
                "let k = `${w + 1}x${h}`, s = scaleFor(),",
            )
        },
        "the name of the key or the cache",
    );
    // 4. a nested object in the sampled value.
    refused(
        |s| {
            s.replace(
                "dom = { span: span, top: top, left: left };",
                "dom = { span: r.height / 10, top: r.top, left: r.left, box: { a: 1 } };",
            )
        },
        "not filled by an object or array literal",
    );
    // 5. conditional and compound assignments are not a plain fill.
    for op in ["||=", "&&=", "??="] {
        refused(
            |s| s.replace("dom = { span", &format!("dom {op} {{ span")),
            "not filled by an object or array literal",
        );
    }
}

#[test]
fn round_five_review_findings_are_refused() {
    let anchor = "let r = document.body.getBoundingClientRect(), span = r.height / 10, top = r.top + 0, left = r.left + 0;";
    let fill = "dom = { span: span, top: top, left: left };";
    // A class field initializer or static block runs later: a deferred refill.
    refused(
        |s| {
            s.replace(
            fill,
            &format!("class K {{ f = (dom = {{ span: span, top: top, left: left }}); }} document.K = K; {fill}"),
        )
        },
        "contains a class",
    );
    // Fill values that are not arithmetic, a literal or a bare qualifying local.
    for value in ["document.body", "(0, span)", "span || 0", "[span][0]"] {
        refused(
            |s| {
                s.replace(
                    fill,
                    &format!("dom = {{ span: span, top: top, left: left, live: {value} }};"),
                )
            },
            "not filled by an object or array literal",
        );
    }
    // Aliases of module functions are module code too.
    for (alias, call) in [
        ("const g = bump;", "g.call();"),
        ("const u = Object.freeze({ f: bump });", "u.f();"),
        ("const u = (0, { f: bump });", "u.f();"),
    ] {
        refused(
            |s| {
                s.replace(anchor, &format!("{anchor} {call}"))
                    + &format!("function bump() {{ w += 100; }}\n{alias}\n")
            },
            "it mentions",
        );
    }
}

#[test]
fn context_specification_errors_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let source = std::fs::read_to_string(context_fixture().join("source.js")).unwrap();
    std::fs::write(dir.path().join("source.js"), &source).unwrap();
    let base: serde_json::Value =
        serde_json::from_slice(&std::fs::read(context_fixture().join("lift.json")).unwrap())
            .unwrap();
    let attempt = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut spec = base.clone();
        spec["source_sha256"] = fingerprint(source.as_bytes()).into();
        edit(&mut spec);
        std::fs::write(dir.path().join("lift.json"), spec.to_string()).unwrap();
        let _ = std::fs::remove_file(dir.path().join("o.ts"));
        let _ = std::fs::remove_file(dir.path().join("o.json"));
        format!(
            "{:#}",
            lift(
                &dir.path().join("lift.json"),
                &dir.path().join("o.ts"),
                &dir.path().join("o.json")
            )
            .unwrap_err()
        )
    };
    let error = attempt(&|s| s["contexts"]["Inputs"]["fields"][1]["name"] = "width".into());
    assert!(error.contains("repeats field `width`"), "{error}");
    let error = attempt(&|s| s["contexts"]["Inputs"]["fields"][3]["from"] = "settings.".into());
    assert!(error.contains("not a captured variable"), "{error}");
    let error = attempt(&|s| {
        s["contexts"]["Inputs"]["fields"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"from": "settings", "name": "all", "type": "unknown"}));
    });
    assert!(error.contains("overlap"), "{error}");
    let error = attempt(&|s| s["functions"][3]["context"]["type"] = "Missing".into());
    assert!(error.contains("does not declare"), "{error}");
    let error = attempt(&|s| s["functions"][3]["context"] = serde_json::Value::Null);
    assert!(error.contains("declares a memo but no context"), "{error}");
    let error = attempt(&|s| s["functions"][3]["memo"]["coupled"] = serde_json::json!([]));
    assert!(error.contains("no coupled cache declared"), "{error}");
    let error = attempt(&|s| s["functions"][4]["memo_read"]["via"] = "fitScale".into());
    assert!(
        error.contains("not a memoizing function") || error.contains("other than as a reset"),
        "{error}"
    );
    let error = attempt(&|s| {
        s["functions"][2]["context"] = serde_json::Value::Null;
    });
    assert!(error.contains("reads captured"), "{error}");
    let error = attempt(&|s| {
        s["functions"].as_array_mut().unwrap().remove(2);
    });
    assert!(error.contains("must also name"), "{error}");
    let error = attempt(&|s| s["declarations"][0] = "type Bad = {};".into());
    assert!(error.contains("declarations must be"), "{error}");
}
