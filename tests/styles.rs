//! Style translation end to end on a synthetic capture (no Tailwind needed: the equality
//! check then runs against this crate's own model of the utilities; with a Tailwind package
//! found by `MOCHI_TAILWIND_DIR` or below `.mochi/mochi-exports` it runs against the real one).

use mochi::semantic::tokens::cli;
use mochi::semantic::tokens::styles::run::{StyleOptions, translate};
use serde_json::json;
use std::path::Path;

fn write_capture(dir: &Path) {
    let pages = dir.join("reconstruction").join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    let el = |parent: Option<usize>, tag: &str, class: &str, style: &str| {
        let mut attrs = vec![json!({"name": "class", "value": class})];
        if !style.is_empty() {
            attrs.push(json!({"name": "style", "value": style}));
        }
        json!({"parent": parent, "kind": "element", "tag": tag, "attributes": attrs, "style": 0, "pseudo": {}})
    };
    let page = json!({
        "url": "https://t.test/", "viewport": {"width": 1000, "height": 800}, "color_scheme": "dark",
        "nodes": [
            el(None, "html", "", ""),
            el(Some(0), "body", "", ""),
            el(Some(1), "nav", "nav", ""),
            el(Some(2), "a", "nav__link", ""),
            el(Some(2), "a", "nav__link", ""),
            el(Some(2), "a", "nav__link", "margin-left: 3px"),
        ],
        "styles": {"strings": ["font-size", "16px"], "maps": [[[0, 1]]]},
        "stylesheets": [{"href": null, "rules": [
            ".nav { display: flex; gap: 8px; padding: 16px 24px; background: #111 }",
            ".nav__link { color: #fff; padding: 4px 8px; text-decoration: none; border-radius: 4px }",
            ".nav__link:hover { color: #0af }",
            "@media (max-width: 599px) { .nav { flex-direction: column; padding: 8px } }",
        ]}],
    });
    std::fs::write(pages.join("page-00000-1000x800.json"), page.to_string()).unwrap();
}

#[test]
fn translation_is_proven_equal_and_written_by_the_cli() {
    let dir = tempfile::tempdir().unwrap();
    let capture = dir.path().join("capture");
    write_capture(&capture);
    let page = Path::new("reconstruction/pages/page-00000-1000x800.json");
    let plan = translate(&capture, page, &StyleOptions::standard()).unwrap();
    let r = &plan.report;
    assert_eq!(r.check.different, 0, "{:?}", r.check.samples);
    assert!(r.check.compared > 100, "{}", r.check.compared);
    assert_eq!(r.status, "static_checks_only_not_browser_verified");
    // Equal utility sets share the component's class; the inline-styled link stays inline.
    assert_eq!(plan.classes[&3], plan.classes[&4]);
    assert_ne!(plan.classes[&3], plan.classes[&5]);
    assert!(
        plan.classes[&2].contains("min-[600px]:"),
        "mobile-first width variants: {:?}",
        plan.classes[&2]
    );
    assert!(plan.files["styles.report.md"].contains("Explicit gaps"));

    let out = dir.path().join("out");
    cli::run(cli::Args {
        capture: capture.clone(),
        out: out.clone(),
        min_uses: 2,
        no_merge_rem_px: false,
        allow_differences: false,
        styles: Some(page.to_path_buf()),
        tailwind_dir: None,
    })
    .unwrap();
    assert!(out.join("src/styles/recipes.css").is_file());
    assert!(out.join("src/styles/tokens.css").is_file());
    assert!(out.join("styles.report.json").is_file());
}

// ----- regressions from the adversarial reviews ---------------------------------------------

use mochi::semantic::tokens::styles::run::StylePlan;
use mochi::semantic::tokens::styles::tailwind;

/// Elements `(parent, tag, class)` with one shared computed style, and the given rules.
fn translate_page(
    nodes: &[(Option<usize>, &str, &str)],
    computed: &[(&str, &str)],
    rules: &[&str],
) -> StylePlan {
    let dir = tempfile::tempdir().unwrap();
    let pages = dir.path().join("reconstruction").join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    let mut strings: Vec<String> = Vec::new();
    let mut map: Vec<[usize; 2]> = Vec::new();
    for (k, v) in computed {
        strings.push((*k).to_string());
        strings.push((*v).to_string());
        map.push([strings.len() - 2, strings.len() - 1]);
    }
    let nodes: Vec<serde_json::Value> = nodes
        .iter()
        .map(|(parent, tag, class)| {
            json!({"parent": parent, "kind": "element", "tag": tag,
                   "attributes": [{"name": "class", "value": class}], "style": 0, "pseudo": {}})
        })
        .collect();
    let page = json!({
        "url": "https://t.test/", "viewport": {"width": 1000, "height": 800}, "color_scheme": "dark",
        "nodes": nodes,
        "styles": {"strings": strings, "maps": [map]},
        "stylesheets": [{"href": null, "rules": rules}],
    });
    std::fs::write(pages.join("page-00000-1000x800.json"), page.to_string()).unwrap();
    translate(
        dir.path(),
        Path::new("reconstruction/pages/page-00000-1000x800.json"),
        &StyleOptions::standard(),
    )
    .unwrap()
}

const FONT: &[(&str, &str)] = &[("font-size", "16px")];
const BASE: &[(Option<usize>, &str, &str)] = &[(None, "html", ""), (Some(0), "body", "")];

fn with(extra: &[(Option<usize>, &str, &str)]) -> Vec<(Option<usize>, String, String)> {
    BASE.iter()
        .chain(extra)
        .map(|(p, t, c)| (*p, (*t).to_string(), (*c).to_string()))
        .collect()
}

fn run(
    extra: &[(Option<usize>, &str, &str)],
    computed: &[(&str, &str)],
    rules: &[&str],
) -> StylePlan {
    let owned = with(extra);
    let nodes: Vec<(Option<usize>, &str, &str)> = owned
        .iter()
        .map(|(p, t, c)| (*p, t.as_str(), c.as_str()))
        .collect();
    translate_page(&nodes, computed, rules)
}

fn verified(plan: &StylePlan) {
    let r = &plan.report;
    assert_eq!(r.check.different, 0, "{:?}", r.check.samples);
    assert_eq!(
        r.status, "static_checks_only_not_browser_verified",
        "{:?}",
        r.untranslated.unresolved_variables
    );
    assert!(r.check.compared > 0);
}

#[test]
fn theme_host_selector_is_not_undecidable_and_variables_are_defined() {
    let p = run(
        &[(Some(1), "div", "a")],
        FONT,
        &[
            "@layer theme { :root, :host { --c: red } }",
            "body.x { --c: blue }",
            ".a { color: var(--c) }",
        ],
    );
    verified(&p);
    assert!(
        p.report.untranslated.undecidable_properties.is_empty(),
        "{:?}",
        p.report.untranslated
    );
    assert!(p.report.untranslated.unresolved_variables.is_empty());
    assert!(p.classes[&0].contains("[--c:red]"), "{}", p.classes[&0]);
    assert_eq!(p.report.check.skipped_uncertain, 0);
}

#[test]
fn all_and_shorthand_aliases_go_to_plain_css_not_utilities() {
    let p = run(
        &[(Some(1), "button", "btn"), (Some(1), "div", "e")],
        FONT,
        &[
            ".btn { all: unset; color: red; cursor: pointer; padding: 4px; display: inline-flex }",
            ".e { align-items: center; place-items: end }",
        ],
    );
    verified(&p);
    let css = &p.files["src/styles/recipes.css"];
    assert!(css.contains("all: unset"), "{css}");
    assert!(css.contains("place-items: end"), "{css}");
    assert!(
        p.report.recipes.fallback_hooks >= 2,
        "{:?}",
        p.report.recipes
    );
}

#[test]
fn focus_within_and_focus_chains_are_translated_with_their_own_variants() {
    let p = run(
        &[(Some(1), "div", "box"), (Some(1), "a", "lnk")],
        FONT,
        &[
            ".box:focus-within { color: red }",
            ".lnk:focus { color: green }",
            "@media (min-width: 600px) { .lnk:focus { color: blue } }",
            "@media (max-width: 599px) { .lnk:focus-visible { color: blue } }",
        ],
    );
    verified(&p);
    let classes = &p.classes[&2];
    assert!(
        classes.contains("focus-within:[color:red]") && !classes.contains("focus:"),
        "{classes}"
    );
}

#[test]
fn hostile_tags_do_not_hang_or_break_recipe_names() {
    let started = std::time::Instant::now();
    let p = run(
        &[(Some(1), "x-a.b", ""), (Some(1), "svg:view", "")],
        FONT,
        &[
            "x-a\\.b { background: red; background-color: blue }",
            "svg\\:view { background: red; background-color: blue }",
        ],
    );
    assert!(started.elapsed().as_secs() < 30);
    verified(&p);
    for class in p.classes.values().filter(|c| !c.is_empty()) {
        assert!(
            class
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || " -_[]:()#,.'%".contains(c)),
            "{class}"
        );
    }
}

#[test]
fn recipe_names_that_tailwind_generates_css_for_are_not_used() {
    if tailwind::locate().is_none() || !tailwind::bun_available() {
        eprintln!("skipped: no Tailwind package");
        return;
    }
    let rule =
        ".collapse { display: block; color: red; padding: 4px; margin: 2px; position: relative }";
    let p = run(
        &[(Some(1), "div", "collapse"), (Some(1), "div", "collapse")],
        FONT,
        &[rule],
    );
    verified(&p);
    assert_ne!(
        p.classes[&2], "collapse",
        "Tailwind's `collapse` is visibility: collapse"
    );
}

#[test]
fn unsafe_values_become_plain_css_instead_of_broken_classes() {
    let p = run(
        &[(Some(1), "div", "a")],
        FONT,
        &[
            ".a { background: url(\"a b.png\") no-repeat; color: red; font-family: \"Foo's Sans\", serif; --x: ; padding: 4px }",
        ],
    );
    verified(&p);
    let classes = &p.classes[&2];
    assert!(
        !classes.contains("url(") && !classes.contains("Foo"),
        "{classes}"
    );
    let css = &p.files["src/styles/recipes.css"];
    assert!(css.contains("url(\"a b.png\")"), "{css}");
}

#[test]
fn empty_longhands_from_a_var_shorthand_are_recovered_from_computed_style() {
    let p = run(
        &[(Some(1), "div", "pe")],
        &[
            ("font-size", "16px"),
            ("animation-duration", "0.6s"),
            ("animation-name", "enter"),
        ],
        &[
            ".pe { opacity: 0; animation-duration: ; animation-name: ; animation-delay: var(--d, 0s) }",
        ],
    );
    verified(&p);
    let css = &p.files["src/styles/recipes.css"];
    let all = format!("{} {css}", p.classes[&2]);
    assert!(
        all.contains("animation-duration:0.6s") || all.contains("animation-duration: 0.6s"),
        "{all}"
    );
    assert!(all.contains("enter"), "{all}");
    assert!(
        p.report
            .untranslated
            .recovered_from_computed
            .contains_key("animation-duration")
    );
    // No output stylesheet contains an empty-valued standard declaration.
    for (path, content) in &p.files {
        if path.ends_with(".css") {
            for line in content.lines() {
                let t = line.trim();
                if let Some((name, value)) = t.split_once(':')
                    && !name.starts_with("--")
                    && !name.contains(' ')
                    && !name.contains('{')
                {
                    assert!(
                        !value.trim().trim_end_matches(';').trim().is_empty(),
                        "{path}: {t}"
                    );
                }
            }
        }
    }
}

#[test]
fn registered_properties_with_an_initial_value_are_defined() {
    let p = run(
        &[(Some(1), "div", "a")],
        FONT,
        &[
            "@property --p { syntax: \"<number>\"; inherits: false; initial-value: 1 }",
            ".a { opacity: calc(var(--p) * 0.5) }",
        ],
    );
    verified(&p);
    assert!(p.report.untranslated.unresolved_variables.is_empty());
}

#[test]
fn variables_the_site_never_defines_do_not_make_the_check_incomplete() {
    let p = run(
        &[(Some(1), "div", "a")],
        FONT,
        &[".a { color: var(--nowhere); padding: 4px }"],
    );
    verified(&p);
    assert!(p.report.untranslated.unresolved_variables.is_empty());
    assert_eq!(p.report.untranslated.undefined_in_original, ["--nowhere"]);
    assert!(p.files["styles.report.md"].contains("never defined"));
}

#[test]
fn shorthand_and_longhand_families_keep_author_order_in_plain_css() {
    let p = run(
        &[
            (Some(1), "div", "x"),
            (Some(1), "div", "y"),
            (Some(1), "div", "z"),
        ],
        FONT,
        &[
            ".x { background: url(a.png) no-repeat; background-position: center }",
            ".y { font: 16px serif; font-variant-numeric: tabular-nums }",
            ".z { grid-row-start: 2; grid-row: 1 / 3 }",
        ],
    );
    verified(&p);
    let css = &p.files["src/styles/recipes.css"];
    for needle in [
        "background: url(a.png) no-repeat;\n  background-position: center",
        "font: 16px serif;\n  font-variant-numeric: tabular-nums",
        "grid-row-start: 2;\n  grid-row: 1 / 3",
    ] {
        assert!(css.contains(needle), "{needle}\n{css}");
    }
    assert!(p.report.recipes.fallback_hooks >= 3);
}

#[test]
fn width_thresholds_in_points_and_inches_are_sampled() {
    let p = run(
        &[(Some(1), "div", "x")],
        FONT,
        &[".x { display: block } @media (min-width: 600pt) { .x { display: none } }"],
    );
    verified(&p);
    assert!(
        p.report.check.widths.iter().any(|w| *w < 800),
        "{:?}",
        p.report.check.widths
    );
    assert!(
        p.classes[&2].contains("min-[800px]:hidden"),
        "{}",
        p.classes[&2]
    );
}

#[test]
fn dropped_undecidable_properties_and_has_states_are_not_reported_as_verified() {
    let p = run(
        &[(Some(1), "div", "x"), (Some(1), "div", "card")],
        FONT,
        &[
            ".x { display: grid; color: red } @container (min-width: 400px) { .x { display: none } }",
            ".card:has(a:hover) { color: blue }",
        ],
    );
    assert_eq!(p.report.status, "static_check_incomplete");
    assert!(
        p.report
            .untranslated
            .undecidable_properties
            .contains_key("display")
    );
    assert!(
        p.report
            .untranslated
            .state_rules_on_ancestors_or_pseudo_elements
            >= 1,
        "{:?}",
        p.report.untranslated
    );
}

#[test]
fn numbered_recipe_names_avoid_tailwind_utilities() {
    if tailwind::locate().is_none() || !tailwind::bun_available() {
        eprintln!("skipped: no Tailwind package");
        return;
    }
    // Three different style groups on the class `order`: the numbered fallback `order-2` is a
    // Tailwind utility (`order: 2`) and must be skipped.
    let p = run(
        &[
            (Some(1), "div", "order"),
            (Some(1), "div", "order"),
            (Some(1), "span", "order"),
            (Some(1), "span", "order"),
            (Some(1), "p", "order"),
            (Some(1), "p", "order"),
        ],
        FONT,
        &[
            "div.order { color: red; padding: 4px; display: block }",
            "span.order { color: blue; padding: 4px; display: block }",
            "p.order { color: green; padding: 4px; display: block }",
        ],
    );
    verified(&p);
    for name in p.classes.values() {
        assert!(!name.contains("order-2"), "{name}");
    }
}

#[test]
fn script_revealed_elements_are_written_in_their_end_state() {
    // The stylesheet hides the element (`opacity: 0; translate: 0 12px`); the capture, taken after
    // the original's script revealed it, computed opacity 1 and no translate.
    let p = run(
        &[(Some(1), "p", "reveal"), (Some(1), "p", "still-hidden")],
        &[
            ("font-size", "16px"),
            ("opacity", "1"),
            ("translate", "none"),
            ("visibility", "visible"),
        ],
        &[
            ".reveal { opacity: 0; translate: 0 12px; visibility: hidden; transition: opacity .2s }",
            ".still-hidden { color: red }",
        ],
    );
    verified(&p);
    let all = format!("{} {}", p.classes[&2], p.files["src/styles/recipes.css"]);
    assert!(
        !all.contains("opacity-0") && !all.contains("opacity:0") && !all.contains("opacity: 0"),
        "{all}"
    );
    assert!(
        all.contains("opacity-100") || all.contains("opacity:1"),
        "{all}"
    );
    assert!(
        all.contains("translate:none") || all.contains("translate: none"),
        "{all}"
    );
    assert!(
        all.contains("visibility:visible") || all.contains("visible"),
        "{all}"
    );
    assert!(
        all.contains("transition"),
        "the animation/transition declarations are kept: {all}"
    );
    assert_eq!(p.report.untranslated.settled_from_computed["opacity"], 1);
    assert!(p.files["styles.report.md"].contains("End states written"));
}

#[test]
fn reduced_motion_overrides_become_motion_reduce_variants() {
    let p = run(
        &[(Some(1), "div", "card"), (Some(1), "div", "plain")],
        &[
            ("font-size", "16px"),
            ("transition-duration", "0.15s"),
            ("animation-name", "enter"),
        ],
        &[
            "@keyframes enter { from { opacity: 0 } to { opacity: 1 } }",
            ".card { transition: opacity 0.15s; animation: enter 0.3s both }",
            "@media (prefers-reduced-motion: reduce) { .card { transition-duration: 1e-05s; animation-duration: 1e-05s } }",
            "@media (prefers-reduced-motion: no-preference) { .plain { animation: enter 1s linear 1 } }",
            ".plain { color: red }",
        ],
    );
    verified(&p);
    let all = format!(
        "{} {} {}",
        p.classes[&2], p.classes[&3], p.files["src/styles/recipes.css"]
    );
    assert!(
        all.contains("motion-reduce:") || all.contains("prefers-reduced-motion: reduce"),
        "{all}"
    );
    assert!(all.contains("0.00001s"), "{all}");
    assert!(p.files["src/styles/at-rules.css"].contains("@keyframes enter"));
    assert!(
        p.report.check.states.iter().any(|s| s == "motion-reduce"),
        "{:?}",
        p.report.check.states
    );
    assert!(
        !p.report
            .untranslated
            .never_applicable_conditions
            .keys()
            .any(|k| k.contains("reduced-motion")),
        "{:?}",
        p.report.untranslated.never_applicable_conditions
    );
}

type NodeList<'a> = &'a [(Option<usize>, &'a str, &'a str)];

/// Captured viewports of one page: `(width, nodes)`; the widest is the primary.
fn translate_views(views: &[(u32, NodeList)], rules: &[&str]) -> StylePlan {
    let dir = tempfile::tempdir().unwrap();
    let pages = dir.path().join("reconstruction").join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    let mut widest = String::new();
    let mut widest_w = 0;
    for (width, nodes) in views {
        let nodes: Vec<serde_json::Value> = nodes
            .iter()
            .map(|(parent, tag, class)| {
                json!({"parent": parent, "kind": "element", "tag": tag,
                       "attributes": [{"name": "class", "value": class}], "style": 0, "pseudo": {}})
            })
            .collect();
        let page = json!({
            "url": "https://t.test/", "viewport": {"width": width, "height": 800}, "color_scheme": "dark",
            "nodes": nodes,
            "styles": {"strings": ["font-size", "16px"], "maps": [[[0, 1]]]},
            "stylesheets": [{"href": null, "rules": rules}],
        });
        let name = format!("page-00000-{width}x800.json");
        std::fs::write(pages.join(&name), page.to_string()).unwrap();
        if *width > widest_w {
            widest_w = *width;
            widest = name;
        }
    }
    translate(
        dir.path(),
        &Path::new("reconstruction/pages").join(widest),
        &StyleOptions::standard(),
    )
    .unwrap()
}

/// Two captured viewports of one page: `(width, class of the card)`.
fn translate_viewports(views: &[(u32, &str)], rules: &[&str]) -> StylePlan {
    type Owned<'a> = Vec<(u32, Vec<(Option<usize>, &'a str, &'a str)>)>;
    let owned: Owned = views
        .iter()
        .map(|(w, c)| {
            (
                *w,
                vec![
                    (None, "html", ""),
                    (Some(0), "body", ""),
                    (Some(1), "div", *c),
                ],
            )
        })
        .collect();
    let refs: Vec<(u32, NodeList)> = owned.iter().map(|(w, n)| (*w, n.as_slice())).collect();
    translate_views(&refs, rules)
}

#[test]
fn a_narrower_captured_viewport_overrides_its_width_interval() {
    // At 375 a script had added `card--compact`; at 1440 it had not.
    let p = translate_viewports(
        &[(1440, "card"), (375, "card card--compact")],
        &[
            ".card { font-size: 16px; color: red }",
            ".card--compact { font-size: 10px }",
            "@media (min-width: 600px) { .card { margin-top: 1px } }",
        ],
    );
    verified(&p);
    let o = &p.report.untranslated.viewport_overlays;
    assert_eq!(o.len(), 1, "{o:?}");
    assert!(
        o[0].skipped.is_none() && o[0].aligned_elements >= 3,
        "{o:?}"
    );
    assert_eq!(o[0].overridden["font-size"], 1, "{o:?}");
    assert!(p.report.check.overlay_compared > 0);
    let classes = &p.classes[&2];
    let css = &p.files["src/styles/recipes.css"];
    let all = format!("{classes} {css}");
    assert!(all.contains("10px") && all.contains("16px"), "{all}");
    assert!(
        all.contains("min-[600px]") || all.contains("width >= 600px"),
        "{all}"
    );
    assert!(p.files["styles.report.md"].contains("Viewport 375x800"));
}

#[test]
fn a_viewport_in_the_same_width_interval_is_not_used() {
    let p = translate_viewports(
        &[(1440, "card"), (1200, "card card--compact")],
        &[
            ".card { font-size: 16px }",
            ".card--compact { font-size: 10px }",
            "@media (min-width: 600px) { .card { margin-top: 1px } }",
        ],
    );
    verified(&p);
    let o = &p.report.untranslated.viewport_overlays;
    assert!(o[0].skipped.is_some(), "{o:?}");
    assert!(!p.classes[&2].contains("10px"));
}

#[test]
fn a_universal_reduced_motion_reset_is_proven() {
    let p = run(
        &[(Some(1), "div", "card")],
        FONT,
        &[
            ".card { transition: opacity 0.15s }",
            "@media (prefers-reduced-motion: reduce) { *, ::before, ::after { animation-duration: 0.01ms !important; transition-duration: 0.01ms !important } }",
        ],
    );
    verified(&p);
    assert!(
        p.classes[&0].contains("motion-reduce:"),
        "{}",
        p.classes[&0]
    );
}

#[test]
fn reduced_motion_and_viewport_overlays_work_together() {
    let p = translate_viewports(
        &[(1440, "card"), (375, "card card--compact")],
        &[
            ".card { font-size: 16px; transition: opacity 0.2s }",
            ".card--compact { font-size: 10px }",
            "@media (min-width: 600px) { .card { margin-top: 1px } }",
            "@media (prefers-reduced-motion: reduce) { *, ::before, ::after { transition-duration: 0.01ms !important } }",
        ],
    );
    verified(&p);
    assert!(p.report.check.overlay_compared > 0);
}

// ----- regressions from the third adversarial review -----------------------------------------

#[test]
fn identity_at_rest_because_of_a_variable_is_not_a_reveal() {
    let p = run(
        &[(Some(1), "div", "card")],
        &[
            ("font-size", "16px"),
            ("transform", "matrix(1, 0, 0, 1, 0, 0)"),
        ],
        &[
            ".card { transform: translateY(var(--y, 0px)) }",
            ".card:hover { --y: -4px }",
            "@media (max-width: 599px) { .card { --y: 10px } }",
        ],
    );
    verified(&p);
    let all = format!("{} {}", p.classes[&2], p.files["src/styles/recipes.css"]);
    assert!(
        !all.contains("transform:none") && !all.contains("transform: none"),
        "{all}"
    );
    assert!(all.contains("-4px") && all.contains("10px"), "{all}");
    assert!(p.report.untranslated.settled_from_computed.is_empty());
}

#[test]
fn overlay_states_come_from_the_other_captures_own_cascade() {
    // At 375 the link also has `--dim`; `.nav-link:hover` still wins there (higher specificity).
    let p = translate_viewports(
        &[(1440, "nav-link"), (375, "nav-link nav-link--dim")],
        &[
            ".nav-link { opacity: 1 }",
            ".nav-link--dim { opacity: 0.6 }",
            ".nav-link:hover { opacity: 1 }",
            "@media (min-width: 600px) { .nav-link { margin-top: 1px } }",
        ],
    );
    verified(&p);
    let all = format!("{} {}", p.classes[&2], p.files["src/styles/recipes.css"]);
    assert!(all.contains("0.6") || all.contains("opacity-60"), "{all}");
    assert!(all.contains("hover"), "{all}");
}

#[test]
fn reduced_motion_rules_bounded_by_width_are_not_deduplicated_away() {
    let p = run(
        &[(Some(1), "div", "hero")],
        FONT,
        &[
            ".hero { animation: rise 1s both }",
            "@media (prefers-reduced-motion: reduce) and (min-width: 800px) { .hero { animation: none } }",
        ],
    );
    verified(&p);
    let all = format!("{} {}", p.classes[&2], p.files["src/styles/recipes.css"]);
    assert!(
        all.contains("motion-reduce") || all.contains("prefers-reduced-motion"),
        "{all}"
    );
}

#[test]
fn motion_reduce_is_restated_where_a_wider_plain_utility_would_beat_it() {
    let p = run(
        &[(Some(1), "div", "hero")],
        FONT,
        &[
            ".hero { opacity: 0.5 }",
            "@media (min-width: 768px) { .hero { opacity: 0.7 } }",
            "@media (prefers-reduced-motion: reduce) { .hero { opacity: 1 } }",
        ],
    );
    verified(&p);
    let all = format!("{} {}", p.classes[&2], p.files["src/styles/recipes.css"]);
    assert!(all.contains("min-[768px]:motion-reduce:"), "{all}");
}

#[test]
fn a_recipe_block_is_rewritten_when_overlapping_declarations_swap_precedence() {
    let p = run(
        &[(Some(1), "div", "x")],
        FONT,
        &[
            ".x { background: url(a.png) }",
            ".x { background-color: red }",
            "@media (min-width: 768px) { .x { background: url(a.png) } }",
        ],
    );
    verified(&p);
    let css = &p.files["src/styles/recipes.css"];
    let wide = css.split("@media (width >= 768px)").nth(1).expect(css);
    assert!(
        wide.contains("background: url(a.png)") && wide.contains("background-color"),
        "{css}"
    );
}

#[test]
fn siblings_one_capture_has_more_of_are_not_paired_by_order() {
    let three: NodeList = &[
        (None, "html", ""),
        (Some(0), "body", ""),
        (Some(1), "ul", ""),
        (Some(2), "li", "slide"),
        (Some(2), "li", "slide"),
        (Some(2), "li", "slide"),
    ];
    let two: NodeList = &[
        (None, "html", ""),
        (Some(0), "body", ""),
        (Some(1), "ul", ""),
        (Some(2), "li", "slide slide--b"),
        (Some(2), "li", "slide"),
    ];
    let p = translate_views(
        &[(1440, three), (375, two)],
        &[
            ".slide { color: red } .slide--b { color: blue }",
            "@media (min-width: 600px) { .slide { margin-top: 1px } }",
        ],
    );
    verified(&p);
    // The three slides are not paired with two: nothing of the narrow capture is applied to them.
    let o = &p.report.untranslated.viewport_overlays;
    assert!(o[0].aligned_elements <= 3, "{o:?}");
    for node in [3, 4, 5] {
        assert!(!p.classes[&node].contains("blue"), "{}", p.classes[&node]);
    }
}

#[test]
fn reduced_motion_combined_with_a_state_is_reported_as_a_gap() {
    let p = run(
        &[(Some(1), "button", "btn")],
        FONT,
        &["@media (prefers-reduced-motion: no-preference) { .btn:hover { scale: 1.05 } }"],
    );
    assert_eq!(p.report.untranslated.reduced_motion_with_state, 1);
    assert!(p.files["styles.report.md"].contains("combine a state"));
}

// ----- reveal end states (Garden: hero nav, story copy, footer items hidden until a script ran) -----

/// One page with nodes `(parent, tag, [(attr, value)])`, a shared computed style and extra page keys.
type RawNode<'a> = (Option<usize>, &'a str, Vec<(&'a str, &'a str)>);

fn translate_raw(
    nodes: &[RawNode],
    computed: &[(&str, &str)],
    rules: &[&str],
    extra: serde_json::Value,
) -> StylePlan {
    let dir = tempfile::tempdir().unwrap();
    let pages = dir.path().join("reconstruction").join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    let mut strings: Vec<String> = Vec::new();
    let mut map: Vec<[usize; 2]> = Vec::new();
    for (k, v) in computed {
        strings.push((*k).to_string());
        strings.push((*v).to_string());
        map.push([strings.len() - 2, strings.len() - 1]);
    }
    // An attribute `#text` gives the element a text child (added after all elements).
    let mut texts: Vec<serde_json::Value> = Vec::new();
    let mut nodes: Vec<serde_json::Value> = nodes
        .iter()
        .enumerate()
        .map(|(i, (parent, tag, attrs))| {
            if let Some((_, t)) = attrs.iter().find(|(k, _)| *k == "#text") {
                texts.push(json!({"parent": i, "kind": "text", "value": t}));
            }
            let attrs: Vec<serde_json::Value> = attrs
                .iter()
                .filter(|(k, _)| *k != "#text")
                .map(|(k, v)| json!({"name": k, "value": v}))
                .collect();
            json!({"parent": parent, "kind": "element", "tag": tag, "attributes": attrs, "style": 0, "pseudo": {}})
        })
        .collect();
    nodes.extend(texts);
    let mut page = json!({
        "url": "https://t.test/", "viewport": {"width": 1000, "height": 800}, "color_scheme": "dark",
        "nodes": nodes,
        "styles": {"strings": strings, "maps": [map]},
        "stylesheets": [{"href": null, "rules": rules}],
    });
    if let (Some(p), Some(e)) = (page.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            p.insert(k.clone(), v.clone());
        }
    }
    std::fs::write(pages.join("page-00000-1000x800.json"), page.to_string()).unwrap();
    translate(
        dir.path(),
        Path::new("reconstruction/pages/page-00000-1000x800.json"),
        &StyleOptions::standard(),
    )
    .unwrap()
}

fn emitted(p: &StylePlan, node: usize) -> String {
    format!("{} {}", p.classes[&node], p.files["src/styles/recipes.css"])
}

#[test]
fn elements_the_capture_lists_with_an_animation_keep_their_end_state() {
    // Computed opacity 1 is the end value of the hero's entrance animation (`animation_states`).
    let p = translate_raw(
        &[
            (None, "html", vec![("class", "")]),
            (Some(0), "body", vec![("class", "")]),
            (Some(1), "nav", vec![("class", "hero-nav")]),
        ],
        &[("font-size", "16px"), ("opacity", "1")],
        &[".hero-nav { opacity: 0; will-change: opacity }"],
        json!({"animation_states": {"2": {"opacity": "1"}}}),
    );
    verified(&p);
    let all = emitted(&p, 2);
    assert!(
        !all.contains("opacity-0") && !all.contains("opacity:0"),
        "{all}"
    );
}

#[test]
fn once_only_reveals_are_shown_but_toggles_panels_and_click_targets_are_not() {
    let a = |k: &'static str, v: &'static str| (k, v);
    let p = translate_raw(
        &[
            (None, "html", vec![a("class", "")]),
            (Some(0), "body", vec![a("class", "")]),
            // 2: named reveal; 3: has a `.is-revealed` rule that clears the offset.
            (
                Some(1),
                "p",
                vec![a("class", "story"), a("data-story-reveal", "")],
            ),
            (Some(1), "li", vec![a("class", "item")]),
            // 4/5: shown by the hover of an ANCESTOR; 6: visibility panel; 7: pointer-less popup.
            (Some(1), "div", vec![a("class", "cta")]),
            (Some(4), "span", vec![a("class", "arrow")]),
            (Some(1), "div", vec![a("class", "panel")]),
            (Some(1), "div", vec![a("class", "popup")]),
            // 8: class-toggled slide; 9: click-target select; 10: named, centered with a translate.
            (Some(1), "div", vec![a("class", "slide")]),
            (
                Some(1),
                "select",
                vec![a("class", "appearance-none overlay")],
            ),
            (Some(1), "div", vec![a("class", "fade-in centered")]),
        ],
        &[
            ("font-size", "16px"),
            ("opacity", "0"),
            ("translate", "-50% -50%"),
            ("transition-property", "all"),
        ],
        &[
            "[data-story-reveal] { opacity: 0; transition: opacity .2s }",
            ".item { opacity: 0; translate: 0 12px } .item.is-revealed { opacity: 1; translate: none }",
            ".arrow { opacity: 0; translate: .5px .5px; transition: opacity .2s }",
            "@media (hover: hover) { .cta:hover .arrow { opacity: 1 } }",
            ".panel { opacity: 0; visibility: hidden; transition: opacity .2s }",
            ".popup { opacity: 0; pointer-events: none; translate: 0 8px }",
            ".slide { opacity: 0; translate: 30px 0 } .slide.is-active { opacity: 1; translate: none }",
            ".overlay { opacity: 0; appearance: none }",
            ".centered { opacity: 0; translate: -50% -50% }",
        ],
        json!({}),
    );
    let s = &p.report.untranslated;
    assert_eq!(s.reveal_pre_state_elements, 3, "{s:?}");
    let shown = |node: usize| {
        let all = p.classes[&node].clone();
        all.contains("opacity-100") || all.contains("opacity:1")
    };
    for node in [2, 3, 10] {
        assert!(shown(node), "{node}: {}", p.classes[&node]);
    }
    for node in [5, 6, 7, 8, 9] {
        assert!(
            !shown(node),
            "{node} must stay hidden: {}",
            p.classes[&node]
        );
    }
    // The reveal rule's own end value clears the offset; a named fade-in with no end value keeps its centering.
    assert!(
        p.classes[&3].contains("translate:none") || p.classes[&3].contains("translate-none"),
        "{}",
        p.classes[&3]
    );
    assert!(p.classes[&10].contains("-50%"), "{}", p.classes[&10]);
}

#[test]
fn text_visibility_is_counted_with_inherited_visibility_and_opacity() {
    let a = |k: &'static str, v: &'static str| (k, v);
    let p = translate_raw(
        &[
            (None, "html", vec![a("class", "")]),
            (Some(0), "body", vec![a("class", "")]),
            (Some(1), "p", vec![a("class", "shown"), a("#text", "shown")]),
            (Some(1), "p", vec![a("class", "faded"), a("#text", "faded")]),
            (Some(1), "div", vec![a("class", "panel")]),
            (
                Some(4),
                "p",
                vec![a("class", "inside"), a("#text", "inside")],
            ),
            (Some(4), "p", vec![a("class", "back"), a("#text", "back")]),
            (Some(1), "p", vec![a("class", "gone"), a("#text", "gone")]),
            (Some(1), "p", vec![a("class", "blank"), a("#text", "  ")]),
        ],
        &[("font-size", "16px")],
        &[
            ".faded { opacity: 0 } .panel { visibility: hidden } .gone { display: none }",
            ".back { visibility: visible }",
        ],
        json!({}),
    );
    let t = &p.report.untranslated.text_visibility;
    // shown + back (explicit `visibility: visible` inside a hidden panel) are visible; whitespace-only text is not text.
    assert_eq!(
        (
            t.text_elements,
            t.visible,
            t.hidden_by_opacity,
            t.hidden_by_visibility,
            t.hidden_by_display
        ),
        (5, 2, 1, 1, 1),
        "{t:?}"
    );
}

/// The Garden capture of the main checkout (`.mochi/try/garden`; set `MOCHI_GARDEN_CAPTURE` to
/// point elsewhere, and `MOCHI_REQUIRE_GARDEN=1` to fail instead of skip when it is missing): the
/// home page's fade-in copy is shown, while hover toggles and visibility panels stay hidden.
#[test]
fn garden_home_reveals_its_fades_when_the_capture_is_available() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let capture = std::env::var_os("MOCHI_GARDEN_CAPTURE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join(".mochi/try/garden"));
    let page = Path::new("reconstruction/pages/page-00000-1440x900.json");
    if !capture.join(page).is_file() {
        assert!(
            std::env::var_os("MOCHI_REQUIRE_GARDEN").is_none(),
            "no Garden capture at {}",
            capture.display()
        );
        eprintln!("skipped: no Garden capture at {}", capture.display());
        return;
    }
    let p = translate(&capture, page, &StyleOptions::standard()).unwrap();
    let t = &p.report.untranslated.text_visibility;
    eprintln!("{t:?}");
    assert!(t.text_elements > 100, "{t:?}");
    assert!(t.visible >= 25, "{t:?}");
    assert!(t.hidden_by_opacity <= 3, "{t:?}");
    let u = &p.report.untranslated;
    assert!((20..=40).contains(&u.reveal_pre_state_elements), "{u:?}");
    assert_eq!(p.report.check.different, 0, "{:?}", p.report.check.samples);
    // Known toggles stay hidden: the CTA arrows shown by an ancestor's hover.
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(capture.join(page)).unwrap()).unwrap();
    let mut checked = 0;
    for (i, n) in raw["nodes"].as_array().unwrap().iter().enumerate() {
        let class = n["attributes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| a["name"] == "class")
            .and_then(|a| a["value"].as_str())
            .unwrap_or("");
        if class
            .split_whitespace()
            .any(|c| c == "ig-interaction-cta__arrow--on")
            && let Some(classes) = p.classes.get(&i)
        {
            checked += 1;
            let all = emitted(&p, i);
            assert!(!classes.contains("opacity-100"), "{i}: {all}");
        }
    }
    eprintln!("{checked} hover-arrow elements checked");
}

#[test]
fn reveal_rules_apply_only_where_their_whole_selector_could_match() {
    let a = |k: &'static str, v: &'static str| (k, v);
    let p = translate_raw(
        &[
            (None, "html", vec![a("class", "")]),
            (Some(0), "body", vec![a("class", "")]),
            (Some(1), "ul", vec![a("class", "logos")]),
            (Some(2), "li", vec![a("class", "logo")]),
            (Some(1), "ul", vec![a("class", "carousel")]),
            (Some(4), "li", vec![a("class", "logo")]),
            (Some(1), "div", vec![a("class", "share-menu")]),
            (Some(1), "button", vec![a("class", "btn")]),
        ],
        &[
            ("font-size", "16px"),
            ("opacity", "0"),
            ("translate", "0px 12px"),
        ],
        &[
            ".logo { opacity: 0; translate: 0 12px }",
            ".logos.is-revealed .logo { opacity: 1; translate: 0px }",
            ".carousel .logo:not(.is-hidden) { opacity: 1 }",
            ".share-menu { opacity: 0 } .share-menu.is-visible { opacity: 1 }",
            "button:disabled { opacity: .5 } [data-state=open] { opacity: 1 }",
            ".btn { opacity: 0 }",
        ],
        json!({}),
    );
    let shown =
        |n: usize| p.classes[&n].contains("opacity-100") || p.classes[&n].contains("opacity:1");
    // Only the logo under `.logos` is a once-only reveal, with the rule's own end value.
    assert!(shown(3), "{}", p.classes[&3]);
    assert!(
        p.classes[&3].contains("translate:0px") || p.classes[&3].contains("translate-0"),
        "{}",
        p.classes[&3]
    );
    assert!(
        !shown(5),
        "carousel logo must stay hidden: {}",
        p.classes[&5]
    );
    assert!(
        !shown(6),
        "an `.is-visible` popover has no reveal evidence: {}",
        p.classes[&6]
    );
    assert!(!shown(7), "{}", p.classes[&7]);
}
