//! End-to-end run of the token pipeline on a small synthetic capture.

use mochi::semantic::tokens::{cli, pipeline, plan::Options};
use serde_json::json;
use std::path::Path;

fn write_capture(dir: &Path) {
    let pages = dir.join("reconstruction").join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    let nodes = |tag_class: &[(&str, &str, u64)]| -> Vec<serde_json::Value> {
        let mut out = vec![
            json!({"parent": null, "kind": "element", "tag": "html", "attributes": [], "style": 0, "pseudo": {}}),
            json!({"parent": 0, "kind": "element", "tag": "body", "attributes": [], "style": 0, "pseudo": {}}),
        ];
        for (tag, class, style) in tag_class {
            out.push(json!({"parent": 1, "kind": "element", "tag": tag,
                "attributes": [{"name": "class", "value": class}], "style": style, "pseudo": {}}));
        }
        out
    };
    let css = ":root { --ink: #111; --paper: #fff } \
        .card { color: var(--ink); background-color: #fff; padding: 16px; border-radius: 8px } \
        .card__title { color: #111; margin-bottom: 8px } \
        .badge { background-color: #fff; padding: 8px 16px; border-radius: 8px }";
    // Computed styles as a browser would report them for the three elements.
    let strings = [
        "font-size",
        "16px",
        "color",
        "rgb(17, 17, 17)",
        "background-color",
        "rgb(255, 255, 255)",
        "padding-top",
        "16px",
        "border-top-left-radius",
        "8px",
        "--ink",
        "#111",
        "--paper",
        "#fff",
        "margin-bottom",
        "8px",
        "padding-left",
        "padding-right",
        "8px",
    ];
    let maps = json!([
        [[0, 1], [10, 11], [12, 13]],
        [[0, 1], [2, 3], [4, 5], [6, 7], [8, 9], [10, 11], [12, 13]],
        [[0, 1], [2, 3], [14, 15]],
        [[0, 1], [4, 5], [6, 18], [8, 9], [16, 7], [17, 7]],
    ]);
    for (width, height) in [(1000u32, 800u32), (400, 800)] {
        let page = json!({
            "url": "https://fixture.test/", "viewport": {"width": width, "height": height}, "color_scheme": "dark",
            "nodes": nodes(&[("div", "card", 1), ("h2", "card__title", 2), ("span", "badge", 3)]),
            "styles": {"strings": strings, "maps": maps},
            "stylesheets": [{"href": "https://fixture.test/assets/app-AbCd1234.css", "rules": [css], "media": ""}],
        });
        std::fs::write(
            pages.join(format!("page-00000-{width}x{height}.json")),
            page.to_string(),
        )
        .unwrap();
    }
}

#[test]
fn tokenizes_a_capture_and_checks_equality() {
    let dir = tempfile::tempdir().unwrap();
    let capture = dir.path().join("capture");
    write_capture(&capture);
    let output = pipeline::run(&capture, &Options::default()).unwrap();

    let tokens = &output.files["src/styles/tokens.css"];
    assert_eq!(tokens.matches("@theme static {").count(), 1, "{tokens}");
    assert!(
        tokens.contains("--ink: #111;") && tokens.contains("--paper: #fff;"),
        "site variables are hoisted:\n{tokens}"
    );
    assert!(tokens.contains("--radius-"), "{tokens}");
    assert!(tokens.contains("--spacing: 4px;"), "{tokens}");

    let sheet = output
        .files
        .iter()
        .find(|(path, _)| path.starts_with("src/styles/rewritten/"))
        .map(|(path, css)| (path.clone(), css.clone()))
        .unwrap();
    assert_eq!(
        sheet.0, "src/styles/rewritten/app.css",
        "hash dropped from the file name"
    );
    assert!(
        sheet.1.contains("background-color: var(--paper)"),
        "{}",
        sheet.1
    );
    assert!(
        sheet.1.contains("padding: calc(var(--spacing) * 4)"),
        "{}",
        sheet.1
    );
    assert!(
        !sheet.1.contains("--ink: #111"),
        "definition moved to the central file:\n{}",
        sheet.1
    );

    let report = &output.report;
    assert_eq!(report.status, "static_checks_only_not_browser_verified");
    assert_eq!(report.snapshots.len(), 2);
    for check in &report.rewrite_equality {
        assert!(check.compared > 0);
        assert_eq!(check.different, 0, "{:?}", check.samples);
    }
    for check in &report.resolver_validation {
        assert_eq!(check.mismatched, 0, "{:?}", check.samples);
    }
    assert!(output.files.contains_key("tokens.manifest.json"));
    assert!(output.files["tokens.equality-harness.js"].contains("mochiTokenEquality"));
}

#[test]
fn cli_refuses_an_existing_output_directory_and_writes_a_new_one() {
    let dir = tempfile::tempdir().unwrap();
    let capture = dir.path().join("capture");
    write_capture(&capture);
    let existing = dir.path().join("exists");
    std::fs::create_dir(&existing).unwrap();
    let args = |out: &Path| cli::Args {
        capture: capture.clone(),
        out: out.to_path_buf(),
        min_uses: 2,
        no_merge_rem_px: false,
        allow_differences: false,
        styles: None,
        tailwind_dir: None,
    };
    assert!(cli::run(args(&existing)).is_err());
    let fresh = dir.path().join("fresh");
    cli::run(args(&fresh)).unwrap();
    assert!(fresh.join("src/styles/tokens.css").is_file());
    assert!(fresh.join("tokens.report.md").is_file());
    assert!(
        cli::run(args(&fresh)).is_err(),
        "a second run must not overwrite"
    );
}

// ----- regressions found by the adversarial review ---------------------------------------------

use mochi::semantic::tokens::css;

/// One page: html > body > div.card, every element with the same tiny computed style, plus the
/// given stylesheets (JSON objects: href/media/disabled/rules).
fn write_page(dir: &Path, name: &str, sheets: Vec<serde_json::Value>) {
    let pages = dir.join("reconstruction").join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    let page = json!({
        "url": "https://fixture.test/", "viewport": {"width": 1000, "height": 800}, "color_scheme": "dark",
        "nodes": [
            {"parent": null, "kind": "element", "tag": "html", "attributes": [], "style": 0, "pseudo": {}},
            {"parent": 0, "kind": "element", "tag": "body", "attributes": [], "style": 0, "pseudo": {}},
            {"parent": 1, "kind": "element", "tag": "div", "attributes": [{"name": "class", "value": "card x y"}], "style": 0, "pseudo": {}}
        ],
        "styles": {"strings": ["font-size", "16px"], "maps": [[[0, 1]]]},
        "stylesheets": sheets,
    });
    std::fs::write(pages.join(format!("{name}.json")), page.to_string()).unwrap();
}

#[test]
fn media_and_disabled_sheets_do_not_leak_into_the_central_file() {
    let dir = tempfile::tempdir().unwrap();
    write_page(
        dir.path(),
        "page-00000-1000x800",
        vec![
            json!({"href": "https://t.test/a/app-AbCd1234.css", "media": "", "rules": [".card { padding-top: 16px }"]}),
            json!({"href": "https://t.test/a/print-ZzYy9876.css", "media": "print", "rules": [":root { --accent: #f00 } .x { color: var(--accent) }"]}),
            json!({"href": "https://t.test/a/alt-QqWw5432.css", "media": "", "disabled": true, "rules": [":root { --bg: #0f0 } .y { background-color: var(--bg) }"]}),
        ],
    );
    let output = pipeline::run(dir.path(), &Options::default()).unwrap();
    let tokens = &output.files["src/styles/tokens.css"];
    assert!(
        !tokens.contains("--accent") && !tokens.contains("--bg"),
        "{tokens}"
    );
    let print = output
        .files
        .iter()
        .find(|(p, _)| p.ends_with("/print.css"))
        .expect("print sheet written")
        .1;
    assert!(
        print.starts_with("@media print {"),
        "media condition survives as a wrapper:\n{print}"
    );
    assert!(
        !output.files.keys().any(|p| p.ends_with("/alt.css")),
        "disabled sheets are not part of the page"
    );
    for c in &output.report.rewrite_equality {
        assert_eq!(c.different, 0, "{:?}", c.samples);
    }
}

#[test]
fn a_variable_defined_on_one_page_only_makes_the_run_fail_unless_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let capture = dir.path().join("capture");
    // Page A defines --brand; page B uses it without defining it (so it is invalid there and
    // `color` inherits). A site-wide central file would make it resolve on page B.
    write_page(
        &capture,
        "page-00000-1000x800",
        vec![
            json!({"href": null, "rules": [":root { --brand: #f00 } .card { color: var(--brand) }"]}),
        ],
    );
    write_page(
        &capture,
        "page-00001-1000x800",
        vec![
            json!({"href": null, "rules": ["body { color: #00f } .card { color: var(--brand) }"]}),
        ],
    );
    let output = pipeline::run(&capture, &Options::default()).unwrap();
    assert!(
        output
            .report
            .rewrite_equality
            .iter()
            .any(|c| c.different > 0),
        "{:?}",
        output.report.rewrite_equality
    );
    assert_eq!(output.report.status, "static_differences_found");
    let args = |out: &Path, allow: bool| cli::Args {
        capture: capture.clone(),
        out: out.to_path_buf(),
        min_uses: 2,
        no_merge_rem_px: false,
        allow_differences: allow,
        styles: None,
        tailwind_dir: None,
    };
    let refused = dir.path().join("refused");
    assert!(cli::run(args(&refused, false)).is_err());
    assert!(
        !refused.exists(),
        "nothing is written when differences are found"
    );
    let allowed = dir.path().join("allowed");
    cli::run(args(&allowed, true)).unwrap();
    assert!(allowed.join("tokens.report.json").is_file());
}

#[test]
fn unmodified_source_text_survives_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    write_page(
        dir.path(),
        "page-00000-1000x800",
        vec![json!({"href": null, "rules": [
            "@font-face { font-family: X; src: url(a.woff2) format(\"woff2\"); unicode-range: U+1E00-1EFF, U+2E80-2EFF }",
            ":root { --fade: 1 } @starting-style { :root { --fade: 0 } } .card { opacity: var(--fade); width: calc(1px + 2px) }",
            "foreignObject[data-X=\"A\" s] { color: red }"
        ]})],
    );
    let output = pipeline::run(dir.path(), &Options::default()).unwrap();
    let (_, sheet) = output
        .files
        .iter()
        .find(|(p, _)| p.starts_with("src/styles/rewritten/"))
        .unwrap();
    assert!(
        sheet.contains("unicode-range: U+1E00-1EFF, U+2E80-2EFF"),
        "{sheet}"
    );
    assert!(
        sheet.contains("foreignObject[data-X=\"A\" s]"),
        "selector text is kept exactly:\n{sheet}"
    );
    assert!(
        sheet.contains("@starting-style") && sheet.contains("--fade: 0"),
        "{sheet}"
    );
    assert!(
        sheet.contains("--fade: 1"),
        "a variable with a conditional override stays in place:\n{sheet}"
    );
    assert!(!output.files["src/styles/tokens.css"].contains("--fade"));
}

#[test]
fn hostile_stylesheet_urls_cannot_escape_the_header_comment() {
    let dir = tempfile::tempdir().unwrap();
    write_page(
        dir.path(),
        "page-00000-1000x800",
        vec![
            json!({"href": "https://t.test/a.css?v=*/ body { display: none !important } /*", "rules": [".card { padding-top: 16px }"]}),
        ],
    );
    let output = pipeline::run(dir.path(), &Options::default()).unwrap();
    let central = css::parse_sheet(&output.files["src/styles/tokens.css"]);
    assert_eq!(
        central.nodes.len(),
        1,
        "only the @theme block is live CSS: {central:?}"
    );
}

fn run_css(rules: &[&str]) -> pipeline::Output {
    let dir = tempfile::tempdir().unwrap();
    write_page(
        dir.path(),
        "page-00000-1000x800",
        vec![json!({"href": null, "rules": rules})],
    );
    pipeline::run(dir.path(), &Options::default()).unwrap()
}

fn rewritten(output: &pipeline::Output) -> &String {
    output
        .files
        .iter()
        .find(|(p, _)| p.starts_with("src/styles/rewritten/"))
        .unwrap()
        .1
}

#[test]
fn text_shadow_never_receives_a_box_shadow_token() {
    let output = run_css(&[
        ".d { box-shadow: 0 1px 2px 0 #000 } .e { box-shadow: 0 1px 2px 0 #000 } .card { text-shadow: 0 1px 2px #000 }",
    ]);
    let css = rewritten(&output);
    assert!(css.contains("text-shadow: 0 1px 2px #000"), "{css}");
    assert!(css.contains("box-shadow: var(--shadow-y1-blur2)"), "{css}");
}

#[test]
fn css_wide_keywords_are_never_tokens() {
    let output = run_css(&[
        "html { font-family: Georgia } .card, .x { font-family: initial } .y { font-family: inherit }",
    ]);
    let css = rewritten(&output);
    assert!(
        css.contains("font-family: initial") && css.contains("font-family: inherit"),
        "{css}"
    );
    assert!(!output.files["src/styles/tokens.css"].contains("initial"));
}

#[test]
fn nested_and_fluid_root_font_sizes_disable_rem_px_merging() {
    for css in [
        "html { font-size: 16px; @media (max-width: 600px) { font-size: 14px } } .card { padding-top: 16px; font-size: 1rem } .x { padding-top: 16px; font-size: 1rem }",
        "html { font-size: calc(12px + .4vw) } .card { padding-top: 16px; font-size: 1rem } .x { padding-top: 16px; font-size: 1rem }",
    ] {
        let output = run_css(&[css]);
        assert!(
            output
                .report
                .gaps
                .iter()
                .any(|g| g.contains("root element's font size")),
            "{css}\n{:?}",
            output.report.gaps
        );
        assert_eq!(output.report.summary.merged_across_px_rem, 0);
    }
}

#[test]
fn media_values_that_smuggle_rules_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write_page(
        dir.path(),
        "page-00000-1000x800",
        vec![
            json!({"href": null, "media": "screen{}.a{color:red}@media all", "rules": [".card { color: blue }"]}),
        ],
    );
    assert!(pipeline::run(dir.path(), &Options::default()).is_err());
}

#[test]
fn an_import_under_a_media_condition_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write_page(
        dir.path(),
        "page-00000-1000x800",
        vec![
            json!({"href": null, "media": "print", "rules": ["@import url(\"x.css\");", ".card { color: blue }"]}),
        ],
    );
    assert!(pipeline::run(dir.path(), &Options::default()).is_err());
}

#[test]
fn corrupt_captures_are_errors_not_panics() {
    let dir = tempfile::tempdir().unwrap();
    let pages = dir.path().join("reconstruction").join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    let base = |strings: serde_json::Value,
                maps: serde_json::Value,
                pseudo: serde_json::Value,
                parent: serde_json::Value| {
        json!({
            "url": "https://t.test/", "viewport": {"width": 1000, "height": 800}, "color_scheme": "dark",
            "nodes": [
                {"parent": parent, "kind": "element", "tag": "html", "attributes": [], "style": 0, "pseudo": pseudo},
            ],
            "styles": {"strings": strings, "maps": maps},
            "stylesheets": [],
        })
    };
    let cases = [
        base(json!(["a", "b"]), json!([[[0, 5]]]), json!({}), json!(null)),
        base(
            json!(["a", "b"]),
            json!([[[0, 1]]]),
            json!({"::before": 9}),
            json!(null),
        ),
        base(json!(["a", "b"]), json!([[[0, 1]]]), json!({}), json!(0)),
    ];
    for (i, case) in cases.iter().enumerate() {
        std::fs::write(
            pages.join(format!("page-0000{i}-1000x800.json")),
            case.to_string(),
        )
        .unwrap();
        assert!(
            pipeline::run(dir.path(), &Options::default()).is_err(),
            "case {i}"
        );
        std::fs::remove_file(pages.join(format!("page-0000{i}-1000x800.json"))).unwrap();
    }
}

#[test]
fn a_run_that_compares_nothing_is_not_reported_as_checked() {
    let output = run_css(&[".nothing-matches { padding-top: 16px }"]);
    assert_eq!(output.report.status, "static_check_vacuous");
}

#[test]
fn third_review_regressions() {
    // Raw declarations keep their terminating `;`; escaped names are escaped in the central file.
    let output = run_css(&[
        ".card { --obj:{a:b}; width: 10px }",
        ":root { --a\\:b: 4px } .x { width: 10px }",
    ]);
    let sheet = rewritten(&output);
    assert!(sheet.contains("--obj:{a:b};"), "{sheet}");
    assert!(
        output.files["src/styles/tokens.css"].contains("--a\\:b: 4px;"),
        "{}",
        output.files["src/styles/tokens.css"]
    );
    for c in &output.report.rewrite_equality {
        assert_eq!(c.different, 0, "{:?}", c.samples);
    }
    // Conditional root font sizes through selectors other than plain `html`.
    for css in [
        "@media (min-width: 2000px) { html, body { font-size: 20px } } .card { padding-top: 16px; font-size: 1rem } .x { padding-top: 16px; font-size: 1rem }",
        "html.dark { font-size: 20px } .card { padding-top: 16px; font-size: 1rem } .x { padding-top: 16px; font-size: 1rem }",
        ":root { --fs: 16px; font-size: var(--fs) } .card { padding-top: 16px; font-size: 1rem } .x { padding-top: 16px; font-size: 1rem }",
        "html { font-size: clamp(14px, 2cqi, 20px) } .card { padding-top: 16px; font-size: 1rem } .x { padding-top: 16px; font-size: 1rem }",
    ] {
        let output = run_css(&[css]);
        assert!(
            output
                .report
                .gaps
                .iter()
                .any(|g| g.contains("root element's font size")),
            "{css}"
        );
    }
}

#[test]
fn inline_style_overrides_stop_a_site_variable_from_absorbing_literals() {
    let dir = tempfile::tempdir().unwrap();
    let pages = dir.path().join("reconstruction").join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    let page = json!({
        "url": "https://t.test/", "viewport": {"width": 1000, "height": 800}, "color_scheme": "dark",
        "nodes": [
            {"parent": null, "kind": "element", "tag": "html", "attributes": [], "style": 0, "pseudo": {}},
            {"parent": 0, "kind": "element", "tag": "body", "attributes": [{"name": "style", "value": "--brand:#f00"}], "style": 0, "pseudo": {}},
            {"parent": 1, "kind": "element", "tag": "div", "attributes": [{"name": "class", "value": "a"}], "style": 0, "pseudo": {}}
        ],
        "styles": {"strings": ["font-size", "16px"], "maps": [[[0, 1]]]},
        "stylesheets": [{"href": null, "rules": [":root { --brand: #123456 } .a { color: #123456 } .b { color: #123456 }"]}],
    });
    std::fs::write(pages.join("page-00000-1000x800.json"), page.to_string()).unwrap();
    let output = pipeline::run(dir.path(), &Options::default()).unwrap();
    assert!(
        output
            .report
            .kept_vars
            .iter()
            .any(|k| k.name == "--brand" && k.reason.contains("inline")),
        "{:?}",
        output.report.kept_vars
    );
    assert_eq!(
        output
            .report
            .rewrite_equality
            .iter()
            .map(|c| c.different)
            .sum::<usize>(),
        0,
        "{:?}",
        output.report.rewrite_equality
    );
}
