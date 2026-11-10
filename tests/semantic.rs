//! Semantic IR, script facts, Base UI pattern recognition, repeated-component
//! extraction and the emitters, on small fixtures plus (when present) the saved
//! captures of the three example sites.
use mochi::semantic::PageAnalysis;
use mochi::semantic::components::{self, tree_edit_distance};
use mochi::semantic::emit::{self, EmitOptions};
use mochi::semantic::ir::Doc;
use mochi::semantic::js::{self, EffectFact, Ex, KeyRef, Lit, StateEffect};
use mochi::semantic::patterns::{Primitive, StateModel, recognize};
use std::path::Path;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(Path::new("tests/fixtures/semantic").join(name)).unwrap()
}

fn primitives(doc: &Doc) -> Vec<Primitive> {
    recognize(doc, &[])
        .matches
        .iter()
        .map(|m| m.primitive)
        .collect()
}

// ---- IR ---------------------------------------------------------------------

#[test]
fn roles_names_and_states_resolve() {
    let doc = Doc::from_html(&fixture("controls.html"));
    let by_name = |n: &str| {
        doc.nodes
            .iter()
            .find(|x| x.name.as_deref() == Some(n))
            .unwrap()
    };
    assert_eq!(by_name("Dark mode").role(), Some("switch"));
    assert_eq!(by_name("Country").role(), Some("combobox")); // native <select>
    // A wrapping <label> names its checkbox.
    let agree = doc
        .nodes
        .iter()
        .find(|n| n.attr("name") == Some("agree"))
        .unwrap();
    assert_eq!(agree.name.as_deref(), Some("Agree"));
    assert_eq!(by_name("Save").role(), Some("button"));
    let dialog = Doc::from_html(&fixture("dialog.html"));
    let sheet = dialog
        .nodes
        .iter()
        .find(|n| n.role() == Some("dialog"))
        .unwrap();
    assert!(dialog.inert(sheet.id));
    assert_eq!(sheet.name.as_deref(), Some("Navigation"));
}

#[test]
fn labelledby_beats_aria_label_and_content() {
    let doc = Doc::from_html(
        r#"<h2 id="h">Real name</h2><button aria-labelledby="h" aria-label="no">Text</button>"#,
    );
    let b = doc.nodes.iter().find(|n| n.tag == "button").unwrap();
    assert_eq!(b.name.as_deref(), Some("Real name"));
}

// ---- rule table / matcher -----------------------------------------------------

#[test]
fn every_rule_primitive_is_recognized_on_its_fixture() {
    let doc = Doc::from_html(&fixture("controls.html"));
    let p = primitives(&doc);
    for want in [
        Primitive::ToggleGroup,
        Primitive::Switch,
        Primitive::Checkbox,
        Primitive::RadioGroup,
        Primitive::Combobox,
        Primitive::Select,
        Primitive::Menu,
        Primitive::Tooltip,
        Primitive::Field,
        Primitive::Button,
    ] {
        assert!(p.contains(&want), "{want:?} missing from {p:?}");
    }
}

#[test]
fn unrecognized_controls_are_listed_with_a_reason() {
    let doc = Doc::from_html(&fixture("controls.html"));
    let r = recognize(&doc, &[]);
    let slider = r
        .unrecognized
        .iter()
        .find(|u| u.role.as_deref() == Some("slider"))
        .expect("slider is not mapped");
    assert!(
        slider.reason.contains("no rule accepted"),
        "{}",
        slider.reason
    );
    assert_eq!(
        r.unrecognized.len(),
        1,
        "only the slider: {:?}",
        r.unrecognized
    );
    assert_eq!(r.native, 1, "one plain link");
}

#[test]
fn tabs_link_panels_by_aria_and_keys_come_from_names() {
    let doc = Doc::from_html(&fixture("tabs-aria.html"));
    let r = recognize(&doc, &[]);
    let m = r
        .matches
        .iter()
        .find(|m| m.primitive == Primitive::Tabs)
        .unwrap();
    let tabs: Vec<_> = m.parts.iter().filter(|p| p.part == "Tab").collect();
    assert_eq!(
        tabs.iter()
            .map(|t| t.key.clone().unwrap())
            .collect::<Vec<_>>(),
        ["alpha", "beta", "gamma"]
    );
    assert_eq!(m.parts.iter().filter(|p| p.part == "Panel").count(), 3);
    assert!(
        tabs.iter().all(|t| t.render_as.as_deref() == Some("a")),
        "link tabs render as <a>"
    );
    assert_eq!(
        m.props.get("defaultValue").map(String::as_str),
        Some("alpha")
    );
    assert!(m.gaps.is_empty(), "{:?}", m.gaps);
    assert!(m.confidence >= 0.8, "{}", m.confidence);
    assert!(
        m.state
            == StateModel::Uncontrolled {
                default: Some("alpha".into()),
                observed_from: "the tab with aria-selected=true in the capture".into()
            }
    );
}

#[test]
fn accordion_items_headers_and_defaults() {
    let doc = Doc::from_html(&fixture("accordion.html"));
    let r = recognize(&doc, &[]);
    let m = r
        .matches
        .iter()
        .find(|m| m.primitive == Primitive::Accordion)
        .unwrap();
    assert_eq!(m.parts.iter().filter(|p| p.part == "Item").count(), 3);
    assert_eq!(m.props.get("keepMounted").map(String::as_str), Some("true"));
    assert_eq!(
        m.props.get("defaultValue").map(String::as_str),
        Some("[\"billing\"]")
    );
    // Single vs multiple is not observable without a script: an explicit gap.
    assert!(
        m.gaps.iter().any(|g| g.contains("single vs multiple")),
        "{:?}",
        m.gaps
    );
    let item = m.parts.iter().find(|p| p.part == "Item").unwrap();
    assert_eq!(item.render_as.as_deref(), Some("li"));
}

#[test]
fn popup_trigger_with_haspopup_is_not_an_accordion() {
    let doc = Doc::from_html(
        r#"<button aria-haspopup="true" aria-expanded="false" aria-controls="p">Menu</button><div id="p"><a href="/a">A</a></div>"#,
    );
    assert!(!primitives(&doc).contains(&Primitive::Accordion));
}

// ---- script facts ---------------------------------------------------------------

#[test]
fn updater_functions_become_toggle_and_toggle_key() {
    let src = r#"function A(){const[a,o]=x.useState(!1),[l,c]=x.useState(null);const h=()=>{o(u=>(u&&c(null),!u))};const g=u=>{c(v=>v===u?null:u)};return n.jsx("button",{onClick:h,"aria-expanded":a,onKeyDown:()=>g("k")})}"#;
    let f = js::extract("a.js", src);
    assert_eq!(f.states.len(), 2);
    let site = &f.sites[0];
    let click = &site.handlers[0].effects;
    assert!(
        click.contains(&StateEffect::Toggle { state: 0 }),
        "{click:?}"
    );
    assert!(
        click.contains(&StateEffect::Set {
            state: 1,
            value: Ex::Lit { value: Lit::Null }
        }),
        "{click:?}"
    );
    assert_eq!(
        site.handlers[1].effects,
        [StateEffect::ToggleKey {
            state: 1,
            key: KeyRef::Literal {
                value: Lit::Str("k".into())
            }
        }]
    );
    assert_eq!(
        site.bindings[0],
        ("aria-expanded".to_owned(), Ex::State { state: 0 })
    );
}

#[test]
fn loop_keys_and_index_bindings_are_resolved_through_symbols() {
    let src = r#"const L=["products","features"];function A(){const[l,c]=x.useState(null);return n.jsx("div",{children:L.map((u,i)=>{const S=l===u;return n.jsx("button",{"aria-expanded":S,onClick:()=>c(u)},u)})})}"#;
    let f = js::extract("a.js", src);
    let b = f.sites.iter().find(|s| s.tag == "button").unwrap();
    assert_eq!(
        f.loops[0].items.as_deref(),
        Some(&["products".to_owned(), "features".to_owned()][..])
    );
    let Ex::Eq { left, right } = &b.bindings[0].1 else {
        panic!("{:?}", b.bindings)
    };
    assert!(
        matches!(**left, Ex::State { .. })
            && matches!(
                **right,
                Ex::Key {
                    key: KeyRef::LoopItem { .. }
                }
            )
    );
    assert!(matches!(
        b.handlers[0].effects[0],
        StateEffect::SetKey { .. }
    ));
    assert_eq!(b.loops, [0]);
}

#[test]
fn class_template_tokens_glued_to_expressions_are_only_kept_when_complete() {
    let src = r#"function A({v}){const[a]=x.useState(!1);return n.jsx("div",{className:`sheet${a?" sheet--open":""} card--${v} plain`})}"#;
    let f = js::extract("a.js", src);
    let s = &f.sites[0];
    assert!(
        s.class_tokens.contains(&"sheet".to_owned()),
        "{:?}",
        s.class_tokens
    );
    assert!(s.class_tokens.contains(&"plain".to_owned()));
    assert!(
        !s.class_tokens.iter().any(|t| t.starts_with("card--")),
        "a prefix glued to ${{v}} is not a token"
    );
    assert_eq!(s.class_conditional[0].0, "sheet--open");
}

#[test]
fn hoisted_children_give_parent_child_relations() {
    let src = r#"function A(){let a,b;a=n.jsx("span",{className:"leaf"});b=n.jsxs("div",{className:"wrap",onMouseEnter:()=>{},children:[a]});return b}"#;
    let f = js::extract("a.js", src);
    let leaf = f.sites.iter().position(|s| s.tag == "span").unwrap();
    let wrap = f.sites.iter().position(|s| s.tag == "div").unwrap();
    assert_eq!(f.sites[leaf].parent, Some(wrap));
    assert_eq!(f.sites[wrap].children, [leaf]);
}

#[test]
fn unparsable_bundles_yield_no_facts_and_say_so() {
    let f = js::extract("bad.js", "function (((");
    assert!(f.parse_errors > 0 && f.sites.is_empty());
}

// ---- DOM <-> script, end to end ---------------------------------------------------

fn dialog_analysis() -> PageAnalysis {
    let doc = Doc::from_html(&fixture("dialog.html"));
    let facts = js::extract("dialog-react.js", &fixture("dialog-react.js"));
    PageAnalysis::from_parts("dialog.html", doc, vec![facts])
}

#[test]
fn dialog_state_machine_trigger_and_split_of_behaviors() {
    let a = dialog_analysis();
    let m = a
        .recognition
        .matches
        .iter()
        .find(|m| m.primitive == Primitive::Dialog)
        .unwrap();
    let trigger = m
        .parts
        .iter()
        .find(|p| p.part == "Trigger")
        .expect("trigger recovered from the handler that toggles the state");
    assert_eq!(a.doc.node(trigger.node).tag, "button");
    let js = m.js.as_ref().unwrap();
    assert!(
        js.provided_by_primitive
            .iter()
            .any(|p| p.contains("Escape")),
        "{:?}",
        js.provided_by_primitive
    );
    assert!(
        js.provided_by_primitive
            .iter()
            .any(|p| p.contains("scroll lock")),
        "{:?}",
        js.provided_by_primitive
    );
    assert_eq!(js.app_code.len(), 1, "{:?}", js.app_code);
    assert!(js.app_code[0].contains("(max-width: 900px)"));
    let fact = a.js[0].effects.iter().find_map(|e| match &e.fact {
        EffectFact::MediaQuery { when_matches, .. } => Some(*when_matches),
        _ => None,
    });
    assert_eq!(
        fact,
        Some(Some(false)),
        "the guard `e.matches || close()` closes only once the query stops matching"
    );
    assert!(matches!(m.state, StateModel::Controlled { .. }));
}

#[test]
fn dialog_is_emitted_controlled_with_the_media_query_guard_and_the_trigger_label() {
    let a = dialog_analysis();
    let e = emit::emit(&a, &EmitOptions::default());
    let c = e
        .components
        .iter()
        .find(|c| c.primitive == Primitive::Dialog)
        .unwrap();
    assert_eq!(c.name, "NavSheet");
    let src = &e.files[&c.file];
    assert!(
        src.contains("const [open, setOpen] = useState(false)"),
        "{src}"
    );
    assert!(
        src.contains("if (!event.matches) { setOpen(false) }"),
        "{src}"
    );
    assert!(
        src.contains("aria-label={open ? 'Close menu' : 'Open menu'}"),
        "{src}"
    );
    // Escape and the scroll lock are the primitive's job.
    assert!(
        !src.contains("Escape") && !src.contains("overflow"),
        "{src}"
    );
    assert!(src.contains("<DialogPortal keepMounted>"));
}

// ---- emitters ---------------------------------------------------------------------

#[test]
fn base_ui_is_imported_only_from_components_ui_and_names_are_semantic() {
    for (name, doc) in [
        ("tabs", Doc::from_html(&fixture("tabs-aria.html"))),
        ("accordion", Doc::from_html(&fixture("accordion.html"))),
    ] {
        let a = PageAnalysis::from_parts(name, doc, Vec::new());
        let e = emit::emit(&a, &EmitOptions::default());
        assert!(
            !e.components.is_empty(),
            "{name}: nothing emitted, skipped: {:?}",
            e.skipped
        );
        for (path, text) in &e.files {
            let imports_base_ui = text.contains("@base-ui/react");
            assert_eq!(
                imports_base_ui,
                path.starts_with("src/components/ui/"),
                "{path}"
            );
            for bad in ["Component1", "Component2", "prop_1", "prop1:", "Item1"] {
                assert!(!text.contains(bad), "{path} contains {bad}");
            }
        }
    }
}

#[test]
fn tabs_composition_carries_typed_data_and_the_default() {
    let a = PageAnalysis::from_parts(
        "tabs",
        Doc::from_html(&fixture("tabs-aria.html")),
        Vec::new(),
    );
    let e = emit::emit(&a, &EmitOptions::default());
    let c = e
        .components
        .iter()
        .find(|c| c.primitive == Primitive::Tabs)
        .unwrap();
    assert_eq!(c.name, "LatestModelsTabs");
    let src = &e.files[&c.file];
    assert!(src.contains("defaultValue = 'alpha'"), "{src}");
    assert!(src.contains("render={<a href={tab.href} />}"), "{src}");
    let data = &e.files[c.data_file.as_ref().unwrap()];
    assert!(
        data.contains("{ value: 'beta', label: 'Beta', href: '/b' }"),
        "{data}"
    );
    assert!(data.contains("export type LatestModelsTabsValue"));
    // Tabs.Root wraps the list and the panels are the caller's children only when absent.
    assert!(src.contains("<TabsRoot"), "{src}");
    assert!(
        e.files["src/components/ui/tabs.tsx"]
            .contains("import { Tabs as Primitive } from '@base-ui/react/tabs'")
    );
}

#[test]
fn unnamed_patterns_are_skipped_with_a_reason_not_given_a_generated_name() {
    // No data-component, no usable class and no accessible name on the root.
    let doc = Doc::from_html(
        r#"<div role="tablist"><button role="tab" aria-selected="true">A</button><button role="tab" aria-selected="false">B</button></div>"#,
    );
    let a = PageAnalysis::from_parts("x", doc, Vec::new());
    let e = emit::emit(&a, &EmitOptions::default());
    assert!(e.components.is_empty());
    assert!(
        e.skipped
            .iter()
            .any(|s| s.reason.contains("no semantic name")),
        "{:?}",
        e.skipped
    );
}

// ---- repeated components ------------------------------------------------------------

const CARDS: &str = r#"<section data-component="Team">
<a class="card" data-component="PersonCard" href="/ana"><img src="/ana.png" alt="Ana"><h3>Ana</h3><p>CEO</p></a>
<a class="card" data-component="PersonCard" href="/bo"><img src="/bo.png" alt="Bo"><h3>Bo</h3><p>CTO</p></a>
<a class="card" data-component="PersonCard" href="/cy"><img src="/cy.png" alt="Cy"><h3>Cy</h3><p>COO</p></a></section>"#;

#[test]
fn repeated_cards_become_a_named_component_with_semantic_props() {
    let doc = Doc::from_html(CARDS);
    let comps = components::extract(&doc);
    let g = comps
        .groups
        .iter()
        .find(|g| g.component_name.as_deref() == Some("PersonCard"))
        .expect("PersonCard");
    assert_eq!(g.instances.len(), 3);
    assert!(g.extractable, "{:?}", g.gaps);
    let mut names: Vec<_> = g.slots.iter().map(|s| s.name.clone().unwrap()).collect();
    names.sort();
    assert_eq!(names, ["alt", "description", "href", "src", "title"]);
}

#[test]
fn group_components_replace_instances_and_typecheck_shape() {
    let a = PageAnalysis::from_parts("cards", Doc::from_html(CARDS), Vec::new());
    let e = emit::emit(&a, &EmitOptions::default());
    let g = e
        .group_components
        .iter()
        .find(|g| g.name == "PersonCard")
        .unwrap();
    let src = &e.files[&g.file];
    assert!(src.contains("export interface PersonCardProps"), "{src}");
    assert!(
        src.contains("{title}") && src.contains("href={href}"),
        "{src}"
    );
}

#[test]
fn anonymous_repeats_are_reported_but_not_extracted() {
    let doc = Doc::from_html(
        r#"<div><div><span>a</span><b>1</b></div><div><span>b</span><b>2</b></div><div><span>c</span><b>3</b></div></div>"#,
    );
    let comps = components::extract(&doc);
    assert!(!comps.groups.is_empty());
    assert!(
        comps
            .groups
            .iter()
            .all(|g| !g.extractable && g.component_name.is_none())
    );
}

#[test]
fn utility_classes_never_name_a_component() {
    for c in [
        "flex",
        "w-24",
        "space-y-4",
        "md:grid",
        "m-py29fx",
        "appearance-none",
        "inline-block",
    ] {
        assert!(components::is_utility_class(c), "{c}");
    }
    for c in ["mobile-nav-sheet", "card__title", "ig-bloom-index"] {
        assert!(!components::is_utility_class(c), "{c}");
    }
}

#[test]
fn tree_edit_distance_is_exact_on_small_trees() {
    let doc = Doc::from_html(
        r#"<div id=a><p>x</p><p>y</p></div><div id=b><p>x</p></div><div id=c><p>x</p><p>y</p></div><div id=d><section><p>x</p><p>y</p></section></div>"#,
    );
    let id = |n: &str| {
        doc.nodes
            .iter()
            .find(|x| x.attr("id") == Some(n))
            .unwrap()
            .id
    };
    assert_eq!(tree_edit_distance(&doc, id("a"), id("c")).0, 0);
    assert_eq!(
        tree_edit_distance(&doc, id("a"), id("b")).0,
        2,
        "a <p> and its text node deleted"
    );
    assert_eq!(
        tree_edit_distance(&doc, id("a"), id("d")).0,
        1,
        "one wrapper inserted"
    );
}

// ---- regressions from the adversarial review --------------------------------------------

fn analysis(html: &str, js_files: &[&str]) -> PageAnalysis {
    let facts = js_files
        .iter()
        .map(|f| js::extract(f, &fixture(f)))
        .collect();
    PageAnalysis::from_parts("fixture", Doc::from_html(&fixture(html)), facts)
}

#[test]
fn hover_navigation_keeps_the_links_beside_its_items() {
    let a = analysis("nav-hover.html", &["nav-hover.js"]);
    let m = a
        .recognition
        .matches
        .iter()
        .find(|m| m.primitive == Primitive::NavigationMenu)
        .expect("hover menu");
    assert_eq!(m.parts.iter().filter(|p| p.part == "Item").count(), 2);
    let e = emit::emit(&a, &EmitOptions::default());
    let c = e
        .components
        .iter()
        .find(|c| c.primitive == Primitive::NavigationMenu)
        .unwrap_or_else(|| panic!("{:?}", e.skipped));
    let src = &e.files[&c.file];
    assert!(
        src.contains("href=\"/pricing\""),
        "the Pricing link must survive: {src}"
    );
    assert!(
        src.contains("NavigationMenuItem value=\"products\"")
            || src.contains("NavigationMenuItem value='products'"),
        "{src}"
    );
    assert!(
        src.contains("Key = (typeof"),
        "the key type must not share a ui wrapper's name: {src}"
    );
}

#[test]
fn controlled_accordion_still_defaults_to_the_item_the_capture_shows_open() {
    let a = analysis("accordion-index.html", &["accordion-index.js"]);
    let m = a
        .recognition
        .matches
        .iter()
        .find(|m| m.primitive == Primitive::Accordion)
        .expect("accordion");
    let StateModel::Controlled {
        initial, observed, ..
    } = &m.state
    else {
        panic!("{:?}", m.state)
    };
    assert_eq!(
        initial.as_deref(),
        Some("0"),
        "the bundle's literal as written (an index); the key the capture shows open is separate"
    );
    assert_eq!(observed.as_deref(), Some("one"));
    assert!(
        m.gaps.iter().any(|g| g.contains("never closes")),
        "{:?}",
        m.gaps
    );
    let e = emit::emit(&a, &EmitOptions::default());
    let c = e
        .components
        .iter()
        .find(|c| c.primitive == Primitive::Accordion)
        .unwrap_or_else(|| panic!("{:?}", e.skipped));
    let src = &e.files[&c.file];
    assert!(
        src.contains("defaultValue = ['one']"),
        "a controlled accordion still needs its initial value: {src}"
    );
}

#[test]
fn a_disclosure_whose_panel_is_far_away_does_not_wrap_the_trigger_in_root() {
    let filler = "<div><span>x</span></div>".repeat(220);
    let html = format!(
        r#"<header><button class="burger" aria-label="Open Menu" aria-expanded="false" aria-controls="menu">m</button></header>{filler}<div id="menu" class="menu-panel"><a href="/a">A</a></div>"#
    );
    let a = PageAnalysis::from_parts("far", Doc::from_html(&html), Vec::new());
    let e = emit::emit(&a, &EmitOptions::default());
    let c = e
        .components
        .iter()
        .find(|c| c.primitive == Primitive::Collapsible)
        .unwrap_or_else(|| panic!("{:?}", e.skipped));
    let src = &e.files[&c.file];
    let root_line = src
        .lines()
        .find(|l| l.contains("<CollapsibleRoot"))
        .unwrap();
    assert!(
        !root_line.contains("burger"),
        "Root must not borrow the trigger's classes: {src}"
    );
    assert!(
        c.gaps.iter().any(|g| g.contains("distant ancestor")),
        "{:?}",
        c.gaps
    );
}

#[test]
fn link_tabs_are_not_native_buttons_and_panels_are_emitted() {
    let a = PageAnalysis::from_parts(
        "tabs",
        Doc::from_html(&fixture("tabs-aria.html")),
        Vec::new(),
    );
    let e = emit::emit(&a, &EmitOptions::default());
    let c = e
        .components
        .iter()
        .find(|c| c.primitive == Primitive::Tabs)
        .unwrap();
    let src = &e.files[&c.file];
    assert!(src.contains("nativeButton={false}"), "{src}");
    assert!(
        src.contains("<TabsPanel") && src.contains("value=\"beta\"")
            || src.contains("value='beta'"),
        "{src}"
    );
    assert!(
        src.contains("keepMounted"),
        "panels the capture keeps while unselected stay mounted: {src}"
    );
    assert!(
        !src.contains("{children}"),
        "every tab has its panel, no children slot: {src}"
    );
}

#[test]
fn state_attributes_survive_on_content_that_is_not_a_part() {
    // A tab set nested in an accordion panel is composed by name, and an inert panel elsewhere keeps `inert`.
    let html = r#"<ul class="outer"><li><button type="button" aria-expanded="true" aria-controls="o1">Open</button><section id="o1" aria-label="Open">
<div class="models"><div role="tablist" aria-label="Models"><button role="tab" aria-selected="true" aria-controls="m0">A</button><button role="tab" aria-selected="false" aria-controls="m1">B</button></div>
<div role="tabpanel" id="m0"><p>a</p></div><div role="tabpanel" id="m1" inert><p>b</p></div></div></section></li>
<li><button type="button" aria-expanded="false" aria-controls="o2">Closed</button><section id="o2" aria-label="Closed" inert><p aria-selected="false" hidden>kept</p></section></li></ul>"#;
    let a = PageAnalysis::from_parts("nest", Doc::from_html(html), Vec::new());
    let e = emit::emit(&a, &EmitOptions::default());
    let acc = e
        .components
        .iter()
        .find(|c| c.primitive == Primitive::Accordion)
        .unwrap_or_else(|| panic!("{:?}", e.skipped));
    let src = &e.files[&acc.file];
    assert!(
        src.contains("<ModelsTabs />") || src.contains("<ModelsTabs"),
        "the nested tab set is rendered by name: {src}"
    );
    assert!(!src.contains("role=\"tablist\""), "{src}");
    assert!(
        src.contains("aria-selected=\"false\"") && src.contains("hidden"),
        "generic content keeps its state attributes: {src}"
    );
    assert!(src.contains("import { ModelsTabs }"), "{src}");
}

#[test]
fn a_menu_owns_its_trigger_and_is_not_also_a_popover() {
    let doc = Doc::from_html(&fixture("controls.html"));
    let r = recognize(&doc, &[]);
    let menu = r
        .matches
        .iter()
        .find(|m| m.primitive == Primitive::Menu)
        .unwrap();
    assert!(
        menu.parts.iter().any(|p| p.part == "Trigger"),
        "{:?}",
        menu.parts
    );
    assert!(
        !r.matches.iter().any(|m| m.primitive == Primitive::Popover),
        "same popup bound twice"
    );
}

#[test]
fn names_that_would_not_compile_are_never_emitted() {
    // Reserved word as prop, and a leading-zero number that would not round-trip.
    let html = r#"<section data-component="Plans">
<div data-component="Step"><span class="switch">a</span><b class="number">01</b><i class="price">9.90</i></div>
<div data-component="Step"><span class="switch">b</span><b class="number">02</b><i class="price">9.95</i></div></section>"#;
    let comps = components::extract(&Doc::from_html(html));
    let g = comps
        .groups
        .iter()
        .find(|g| g.component_name.as_deref() == Some("Step"))
        .unwrap();
    assert!(
        g.slots.iter().all(|s| s.name.as_deref() != Some("switch")),
        "{:?}",
        g.slots
    );
    assert!(
        !g.extractable,
        "an unnamed slot blocks extraction: {:?}",
        g.gaps
    );
    let number = g.slots.iter().find(|s| s.values == ["01", "02"]).unwrap();
    assert_eq!(number.ts_type, "string", "01 does not print back as 01");
    let price = g
        .slots
        .iter()
        .find(|s| s.values == ["9.90", "9.95"])
        .unwrap();
    assert_eq!(price.ts_type, "string", "9.90 prints back as 9.9");
}

#[test]
fn a_component_never_takes_the_name_of_a_ui_wrapper() {
    let html = r#"<div class="toggle-group"><button aria-pressed="true">A</button><button aria-pressed="false">B</button></div>"#;
    let a = PageAnalysis::from_parts("x", Doc::from_html(html), Vec::new());
    let e = emit::emit(&a, &EmitOptions::default());
    assert!(
        e.components.iter().all(|c| c.name != "ToggleGroup"),
        "{:?}",
        e.components
    );
    assert!(
        e.skipped
            .iter()
            .any(|s| s.reason.contains("no semantic name")),
        "{:?}",
        e.skipped
    );
}

#[test]
fn dependency_effects_need_a_resolved_name_and_an_unconditional_body() {
    let src = r#"function N(e){const[o,s]=x.useState(!1);x.useEffect(()=>{if(e)s(!1)},[e]);const{pathname:p}=useLoc();x.useEffect(()=>{s(!1)},[p]);return n.jsx("div",{})}"#;
    let f = js::extract("n.js", src);
    let deps: Vec<_> = f
        .effects
        .iter()
        .filter_map(|e| match &e.fact {
            EffectFact::DepsChange {
                deps,
                unconditional,
                ..
            } => Some((deps.clone(), *unconditional)),
            _ => None,
        })
        .collect();
    assert!(
        deps.contains(&(vec!["(unresolved)".to_owned()], false)),
        "a minified parameter is not a name, and the body is conditional: {deps:?}"
    );
    assert!(
        deps.contains(&(vec!["pathname".to_owned()], true)),
        "{deps:?}"
    );
}

#[test]
fn glued_conditional_class_suffixes_are_not_classes() {
    let src = r#"function A(){const[o]=x.useState(!1);return n.jsx("div",{className:`menu--${o?"open":"closed"} a${o?" a--on":""}`})}"#;
    let f = js::extract("a.js", src);
    let toks: Vec<&str> = f.sites[0]
        .class_conditional
        .iter()
        .map(|(t, _)| t.as_str())
        .collect();
    assert_eq!(
        toks,
        ["a--on"],
        "`open`/`closed` are suffixes of menu--: {toks:?}"
    );
}

#[test]
fn toggling_between_a_literal_and_null_is_a_toggle_key() {
    let f = js::extract(
        "a.js",
        r#"function A(){const[v,s]=x.useState(null);return n.jsx("button",{onClick:()=>s(c=>c==="a"?null:"a")})}"#,
    );
    assert!(
        matches!(
            f.sites[0].handlers[0].effects[0],
            StateEffect::ToggleKey { .. }
        ),
        "{:?}",
        f.sites[0].handlers
    );
}

#[test]
fn scroll_lock_only_counts_when_the_open_state_hides_overflow() {
    let hide = js::extract(
        "a.js",
        r#"function A(){const[o]=x.useState(!1);x.useEffect(()=>{document.body.style.overflow=o?"hidden":""},[o])}"#,
    );
    let inverted = js::extract(
        "a.js",
        r#"function A(){const[o]=x.useState(!1);x.useEffect(()=>{document.body.style.overflow=o?"":"hidden"},[o])}"#,
    );
    let locks = |f: &mochi::semantic::js::JsFacts| {
        f.effects
            .iter()
            .filter(|e| matches!(e.fact, EffectFact::ScrollLock { .. }))
            .count()
    };
    assert_eq!((locks(&hide), locks(&inverted)), (1, 0));
}

#[test]
fn near_duplicate_search_stays_bounded_on_a_long_list_of_distinct_siblings() {
    let items: String = (0..120)
        .map(|i| {
            format!(
                "<div class=\"row{i}\">{}</div>",
                (0..30)
                    .map(|j| format!("<p class=\"c{i}x{j}\">t</p>"))
                    .collect::<String>()
            )
        })
        .collect();
    let started = std::time::Instant::now();
    let comps = components::extract(&Doc::from_html(&format!("<main>{items}</main>")));
    assert!(comps.variants.len() <= 4000);
    assert!(
        started.elapsed().as_secs() < 20,
        "took {:?}",
        started.elapsed()
    );
}

// ---- regressions from the second review ----------------------------------------------------

fn emit_html(html: &str) -> (emit::Emitted, PageAnalysis) {
    let a = PageAnalysis::from_parts("x", Doc::from_html(html), Vec::new());
    (emit::emit(&a, &EmitOptions::default()), a)
}

fn component(e: &emit::Emitted, p: Primitive) -> &str {
    let c = e
        .components
        .iter()
        .find(|c| c.primitive == p)
        .unwrap_or_else(|| panic!("{p:?} not emitted: {:?}", e.skipped));
    &e.files[&c.file]
}

#[test]
fn accordion_items_keep_the_content_beside_their_trigger_and_panel() {
    let html = r#"<ul class="nav"><li><div class="row"><a href="/models">Models</a><button type="button" aria-expanded="false" aria-controls="x0">More</button></div><div id="x0" aria-label="Models" inert><p>Latest</p></div></li>
<li><div class="row"><a href="/blog">Blog</a><button type="button" aria-expanded="false" aria-controls="x1">More</button></div><div id="x1" aria-label="Blog" inert><p>Posts</p></div></li></ul>"#;
    let (e, _) = emit_html(html);
    let src = component(&e, Primitive::Accordion);
    assert!(
        src.contains("href=\"/models\"") && src.contains("href=\"/blog\""),
        "{src}"
    );
    assert!(
        src.contains("className=\"row\""),
        "the wrapper div stays: {src}"
    );
    // The captured list and items are kept as `ul`/`li`, not replaced by the part's `div`.
    assert!(src.contains("render={<ul />}"), "{src}");
    assert!(src.contains("render={<li />}"), "{src}");
}

#[test]
fn parts_keep_the_captured_tag_and_tabs_with_icons_keep_their_markup() {
    let nav = emit::emit(
        &PageAnalysis::from_parts(
            "nav",
            Doc::from_html(&fixture("nav-hover.html")),
            vec![js::extract("nav-hover.js", &fixture("nav-hover.js"))],
        ),
        &EmitOptions::default(),
    );
    let c = nav
        .components
        .iter()
        .find(|c| c.primitive == Primitive::NavigationMenu)
        .unwrap_or_else(|| panic!("hover menu not emitted: {:?}", nav.skipped));
    let src = &nav.files[&c.file];
    assert!(src.contains("render={<"), "{src}");
    assert!(
        !src.contains("<NavigationMenuItem>\n"),
        "no Item wrapper for bare links: {src}"
    );
    let tabs = r#"<div role="tablist" class="t" aria-label="Install method"><button role="tab" id="a" aria-selected="true" aria-controls="pa"><svg width="1"><path d="M0"/></svg><span>One</span></button><button role="tab" id="b" aria-selected="false" aria-controls="pb"><svg width="1"><path d="M1"/></svg><span>Two</span></button></div><div role="tabpanel" id="pa" aria-labelledby="a">A</div><div role="tabpanel" id="pb" aria-labelledby="b" hidden>B</div>"#;
    let (e, _) = emit_html(tabs);
    let src = component(&e, Primitive::Tabs);
    assert!(
        src.contains("<svg") && src.contains("<span>One</span>"),
        "{src}"
    );
    assert!(!src.contains(".map("), "{src}");
}

#[test]
fn a_collapsible_keeps_everything_else_inside_its_root() {
    let html = r#"<div class="plans"><h2>Plans overview</h2><button type="button" aria-expanded="false" aria-controls="pp">Plans</button><div id="pp" inert><p>Pro plan</p></div><p>Footer note</p></div>"#;
    let (e, _) = emit_html(html);
    let src = component(&e, Primitive::Collapsible);
    for text in ["Plans overview", "Pro plan", "Footer note"] {
        assert!(src.contains(text), "{text} was dropped: {src}");
    }
}

#[test]
fn two_disclosures_sharing_a_root_are_never_silently_merged() {
    let html = r#"<div class="faq"><button type="button" aria-expanded="false" aria-controls="a">Billing</button><div id="a" inert>x</div><button type="button" aria-expanded="false" aria-controls="b">Security</button><div id="b" inert>y</div></div>"#;
    let (e, _) = emit_html(html);
    assert!(
        e.components.iter().all(|c| c.also_matches.is_empty()),
        "different content must not share a component: {:?}",
        e.components
    );
    assert!(
        e.components.len() + e.skipped.len() >= 2,
        "the second disclosure is emitted or reported: {:?}",
        e.skipped
    );
}

#[test]
fn a_toggle_group_with_several_pressed_buttons_is_multiple() {
    let html = r#"<div data-component="Filters"><button aria-pressed="true">Red</button><button aria-pressed="true">Blue</button><button aria-pressed="false">Green</button></div>"#;
    let (e, _) = emit_html(html);
    let src = component(&e, Primitive::ToggleGroup);
    assert!(src.contains("multiple={true}"), "{src}");
    assert!(
        src.contains("[\"red\", \"blue\"]"),
        "every pressed key is a default: {src}"
    );
    assert!(src.contains("Multiple selection"), "{src}");
}

#[test]
fn dependency_effects_do_not_invent_names_from_aliases_props_or_guarded_bodies() {
    let alias = js::extract(
        "a.js",
        r#"function N(e){const[o,s]=x.useState(!1);const r=e;x.useEffect(()=>{s(!1)},[r]);return n.jsx("div",{})}"#,
    );
    let prop = js::extract(
        "a.js",
        r#"function N(e){const[o,s]=x.useState(!1);const{count:r}=e;x.useEffect(()=>{s(!1)},[r]);return n.jsx("div",{})}"#,
    );
    let guard = js::extract(
        "a.js",
        r#"function N(){const[o,s]=x.useState(!1);const{pathname:r}=useLoc();x.useEffect(()=>{if(o)return;s(!1)},[r]);return n.jsx("div",{})}"#,
    );
    let get = |f: &mochi::semantic::js::JsFacts| {
        f.effects.iter().find_map(|e| match &e.fact {
            EffectFact::DepsChange {
                deps,
                unconditional,
                ..
            } => Some((deps.clone(), *unconditional)),
            _ => None,
        })
    };
    assert_eq!(
        get(&alias).unwrap().0,
        ["(unresolved)"],
        "an alias of a prop"
    );
    assert_eq!(
        get(&prop).unwrap().0,
        ["(unresolved)"],
        "a member of a prop is not a hook call"
    );
    assert_eq!(
        get(&guard).unwrap(),
        (vec!["pathname".to_owned()], false),
        "after an early-return guard the reset is conditional"
    );
}

#[test]
fn bundle_sites_that_close_the_dialog_without_a_captured_element_are_gaps() {
    // The closing `<a>` is in the dialog's own component but is absent from the captured DOM.
    let own = fixture("dialog-react.js").replace(
        "children:\"x\"})",
        "children:n.jsx(\"a\",{className:\"ghost-link\",onClick:()=>{setOpen(!1)}})})",
    );
    let facts2 = js::extract("d.js", &own);
    let a = PageAnalysis::from_parts("d", Doc::from_html(&fixture("dialog.html")), vec![facts2]);
    let e = emit::emit(&a, &EmitOptions::default());
    let c = e
        .components
        .iter()
        .find(|c| c.primitive == Primitive::Dialog)
        .unwrap_or_else(|| panic!("{:?}", e.skipped));
    assert!(
        c.gaps
            .iter()
            .any(|g| g.contains("no captured element is linked")),
        "{:?}",
        c.gaps
    );
}

#[test]
fn tab_labels_use_visible_text_not_image_alt_text() {
    let html = r#"<div class="m"><div role="tablist" aria-label="Models"><a role="tab" href="/a" aria-selected="true"><img src="/a.png" alt="Shield icon"><span>Cyber</span></a><a role="tab" href="/b" aria-selected="false"><img src="/b.png" alt="Code icon"><span>Code</span></a></div></div>"#;
    let (e, _) = emit_html(html);
    let c = e
        .components
        .iter()
        .find(|c| c.primitive == Primitive::Tabs)
        .unwrap();
    let data = &e.files[c.data_file.as_ref().unwrap()];
    assert!(
        data.contains("label: 'Cyber'") && !data.contains("Shield"),
        "{data}"
    );
}

#[test]
fn framework_and_utility_classes_are_kept_and_state_modifiers_are_dropped_only_on_parts() {
    let html = r#"<ul class="nav"><li class="item--open is-open"><button type="button" aria-expanded="false" aria-controls="x0">A</button><div id="x0" inert><div class="is-layout-flex has-text-color is-active:opacity-100 is-open">c</div></div></li>
<li><button type="button" aria-expanded="false" aria-controls="x1">B</button><div id="x1" inert><p>d</p></div></li></ul>"#;
    let (e, _) = emit_html(html);
    let src = component(&e, Primitive::Accordion);
    // Content inside a panel is plain markup: every captured class stays, state-like or not.
    assert!(
        src.contains("is-layout-flex")
            && src.contains("has-text-color")
            && src.contains("is-active:opacity-100"),
        "{src}"
    );
    assert!(
        src.contains("is-open"),
        "plain markup keeps its captured state class: {src}"
    );
    // The accordion item is a part of the composition: the primitive owns its open state.
    let item = src.lines().find(|l| l.contains("<AccordionItem")).unwrap();
    assert!(
        !item.contains("item--open") && !item.contains("is-open"),
        "{item}"
    );
    assert!(
        e.state_classes_dropped.contains("item--open"),
        "{:?}",
        e.state_classes_dropped
    );
}

#[test]
fn a_structural_menu_popup_is_not_also_a_popover() {
    let doc = Doc::from_html(
        r#"<button aria-haspopup="true" aria-expanded="false">Actions</button><div role="menu" aria-label="Actions"><button role="menuitem">Edit</button></div>"#,
    );
    let p = primitives(&doc);
    assert!(
        p.contains(&Primitive::Menu) && !p.contains(&Primitive::Popover),
        "{p:?}"
    );
}

// ---- whole-project assembly -----------------------------------------------------------------

#[test]
fn assembling_natural_gives_a_complete_project_without_a_recovered_bundle() {
    let root = Path::new(".mochi/trials/20261108-normalized/natural/capture");
    if !root.join("manifest.json").exists() {
        eprintln!("skipping: {} not present", root.display());
        return;
    }
    let project = mochi::semantic::project::assemble(
        root,
        &mochi::semantic::project::Options {
            viewport: Some("1440x900"),
            routes: None,
            tokens: false,
            styles: mochi::semantic::project::StyleSource::CapturedSheets,
        },
    )
    .unwrap();
    let files = &project.files;
    let text = |p: &str| String::from_utf8(files[p].clone()).unwrap();
    assert_eq!(project.report.routes.len(), 3);
    for p in [
        "src/routes/index.tsx",
        "src/routes/pay.tsx",
        "src/routes/request.tsx",
        "src/routes/__root.tsx",
        "src/router.tsx",
        "src/pages/home.tsx",
        "package.json",
        "bun.lock",
        "vite.config.ts",
        "tsconfig.json",
        "scripts/generate-route-tree.ts",
        ".mochi/provenance/assets.json",
        ".mochi/semantic/project.report.md",
    ] {
        assert!(files.contains_key(p), "{p} missing");
    }
    assert_eq!(
        text("package.json"),
        include_str!("../src/export_template/package.json")
    );
    assert!(text("src/routes/pay.tsx").contains("createFileRoute('/pay')"));
    // Base UI only in components/ui; no hash or numbered names; no scripts in the markup.
    for (path, bytes) in files {
        let Ok(src) = std::str::from_utf8(bytes) else {
            continue;
        };
        if path.starts_with("src/") && src.contains("@base-ui/react") {
            assert!(path.starts_with("src/components/ui/"), "{path}");
        }
        if path.starts_with("src/pages/") || path.starts_with("src/components/") {
            assert!(
                !src.contains("<script") && !src.contains("dangerouslySetInnerHTML"),
                "{path}"
            );
            for bad in ["Component1", "Component2", "prop_1"] {
                assert!(!src.contains(bad), "{path} contains {bad}");
            }
        }
    }
    // Every copied asset matches its provenance row byte for byte; no script or stylesheet bundle is copied.
    let provenance: serde_json::Value =
        serde_json::from_str(&text(".mochi/provenance/assets.json")).unwrap();
    let rows = provenance["assets"].as_array().unwrap();
    assert!(!rows.is_empty());
    for row in rows {
        let public = format!(
            "public{}",
            percent_decode(row["public_path"].as_str().unwrap())
        );
        let bytes = files
            .get(&public)
            .unwrap_or_else(|| panic!("{public} not copied"));
        use sha2::{Digest, Sha256};
        let hex = |b: &[u8]| -> String {
            Sha256::digest(b)
                .iter()
                .map(|x| format!("{x:02x}"))
                .collect()
        };
        // The copy, the provenance row and the mirrored original (read independently) agree.
        let original = std::fs::read(root.join(row["source"].as_str().unwrap())).unwrap();
        assert_eq!(hex(bytes), row["sha256"].as_str().unwrap(), "{public}");
        assert_eq!(
            hex(&original),
            row["sha256"].as_str().unwrap(),
            "{public} differs from the mirror"
        );
        assert!(
            !public.ends_with(".js") && !public.ends_with(".mjs") && !public.ends_with(".css"),
            "{public}"
        );
    }
    // Every image the emitted markup names is under public/ (the repeated product cards once
    // named files that were never copied while the report said nothing was missing).
    let dangling: Vec<_> = project
        .report
        .assets
        .skipped
        .iter()
        .filter(|s| s.reason.contains("no file is present under public/"))
        .collect();
    assert!(dangling.is_empty(), "{dangling:?}");
    for image in [
        "pay-graphic-1.jpg",
        "pay-graphic-3.jpg",
        "request-graphic-1.jpg",
        "request-graphic-3.jpg",
    ] {
        let public = format!("public/assets/images/products/{image}");
        assert!(files.contains_key(&public), "{public} not copied");
        assert!(
            files.iter().any(|(p, b)| p.starts_with("src/")
                && std::str::from_utf8(b).is_ok_and(|t| t.contains(&public[6..]))),
            "{image} is not referenced by any emitted file"
        );
    }
    // The report keeps the gaps it cannot close.
    let gaps = project.report.gaps.join("\n");
    for needle in ["motion", "graphics", "behavior", "styles"] {
        assert!(gaps.contains(needle), "gap about {needle} missing: {gaps}");
    }
    let inv = &project.report.inventory[0];
    assert!(inv.component_elements > 0 && inv.plain_elements > 0 && inv.controls > 0);
    assert_eq!(
        inv.component_elements + inv.plain_elements,
        inv.body_elements
    );
}

#[test]
fn props_of_a_repeated_component_go_through_the_asset_mapping() {
    // The values a repeated-structure component receives are URLs the project must serve: a
    // slot value that bypasses the mapping leaves the page requesting a file that was never copied.
    let a = PageAnalysis::from_parts("cards", Doc::from_html(CARDS), Vec::new());
    let map = |u: &str| u.strip_suffix(".png").map(|s| format!("/local{s}.webp"));
    let e = emit::emit(
        &a,
        &EmitOptions {
            page: true,
            urls: Some(&map),
            ..EmitOptions::default()
        },
    );
    let jsx = e.page.unwrap().jsx;
    for who in ["bo", "cy"] {
        assert!(jsx.contains(&format!("/local/{who}.webp")), "{jsx}");
        assert!(!jsx.contains(&format!("\"/{who}.png\"")), "{jsx}");
    }
}

/// A bridge that answers for every element, as a style translation does.
struct EveryElement(&'static str);
impl emit::StyleBridge for EveryElement {
    fn class_name(&self, _doc: &Doc, _node: usize, _original: &[String]) -> String {
        self.0.to_owned()
    }
}

#[test]
fn the_style_bridge_is_asked_for_classless_elements_and_script_hooks_survive() {
    let html = r#"<body class="js-body page"><main><p>text</p><button class="js-toggle btn">go</button></main></body>"#;
    let a = PageAnalysis::from_parts("x", Doc::from_html(html), Vec::new());
    let bridge = EveryElement("flex gap-2");
    let e = emit::emit(
        &a,
        &EmitOptions {
            page: true,
            style: &bridge,
            ..EmitOptions::default()
        },
    );
    let jsx = e.page.unwrap().jsx;
    // The class-less <main> and <p> get the translation; the hook stays beside it; the
    // captured name of a styled class does not.
    assert!(jsx.contains("<main className=\"flex gap-2\">"), "{jsx}");
    assert!(jsx.contains("<p className=\"flex gap-2\">"), "{jsx}");
    assert!(jsx.contains("className=\"flex gap-2 js-toggle\""), "{jsx}");
    assert!(!jsx.contains("btn"), "{jsx}");
    // <html>/<body> attributes take the same route.
    let doc = Doc::from_html(html);
    let body = doc.nodes.iter().find(|n| n.tag == "body").unwrap().id;
    let (props, _) = emit::element_props(&doc, body, &bridge);
    assert!(
        props.contains(&("className".to_owned(), "flex gap-2 js-body".to_owned())),
        "{props:?}"
    );
}

#[test]
fn tab_classes_with_quotes_are_valid_jsx_and_the_selected_tab_is_not_the_template() {
    struct ByNode;
    impl emit::StyleBridge for ByNode {
        fn class_name(&self, doc: &Doc, node: usize, _o: &[String]) -> String {
            // The selected tab looks different in the capture; the others carry a quote.
            if doc.node(node).aria_bool("aria-selected") == Some(true) {
                "selected-look".to_owned()
            } else {
                "[content:\"x\"] idle".to_owned()
            }
        }
    }
    let a = PageAnalysis::from_parts(
        "tabs",
        Doc::from_html(&fixture("tabs-aria.html")),
        Vec::new(),
    );
    let e = emit::emit(
        &a,
        &EmitOptions {
            style: &ByNode,
            ..EmitOptions::default()
        },
    );
    let c = e
        .components
        .iter()
        .find(|c| c.primitive == Primitive::Tabs)
        .unwrap();
    let src = &e.files[&c.file];
    assert!(!src.contains("\\\""), "Debug-style escapes in JSX: {src}");
    assert!(src.contains("className={'[content:\"x\"] idle'}"), "{src}");
    assert!(!src.contains("selected-look"), "{src}");
}

#[test]
fn page_markup_keeps_text_and_drops_what_must_not_run() {
    let html = r#"<body>
<img srcset="data:image/svg+xml;base64,PHN2Zz4=, /a.png 2x, /w_1,h_1/b.png 3x" src="/a.png">
<iframe src="javascript:alert(1)"></iframe><next-route-announcer><div role="alert"></div></next-route-announcer>
<pre>a
  b</pre><p>R&amp;D &amp;amp; 10&nbsp;000</p><details open><summary>s</summary></details>
<table border="0" cellpadding="0"></table><input value="x" tabindex=""><div :class="x" @click="y"></div>
<iframe src="/frame"><html><body>inner</body></html></iframe><div class="state--active is-layout-flex"></div></body>"#;
    let a = PageAnalysis::from_parts("x", Doc::from_html(html), Vec::new());
    let map = |u: &str| (u == "/a.png").then(|| "/local/a.png".to_owned());
    let e = emit::emit(
        &a,
        &EmitOptions {
            page: true,
            urls: Some(&map),
            ..EmitOptions::default()
        },
    );
    let page = e.page.unwrap();
    let jsx = &page.jsx;
    assert!(
        !jsx.contains("javascript:") && !jsx.contains("next-route-announcer"),
        "{jsx}"
    );
    assert!(
        page.elements_dropped
            .keys()
            .any(|k| k.contains("javascript:")),
        "{:?}",
        page.elements_dropped
    );
    // srcset: a data URI keeps its comma, a URL with commas is intact, only mapped candidates change.
    assert!(jsx.contains("data:image/svg+xml;base64,PHN2Zz4="), "{jsx}");
    assert!(
        jsx.contains("/local/a.png 2x") && jsx.contains("/w_1,h_1/b.png 3x"),
        "{jsx}"
    );
    // Text: &nbsp; stays text, a literal "&amp;" is escaped, pre keeps its lines.
    assert!(jsx.contains("10\u{a0}000"), "{jsx}");
    assert!(jsx.contains("{'&'}amp;"), "{jsx}");
    assert!(jsx.contains("{'a\\n  b'}"), "{jsx}");
    // Attribute typing: booleans bare, unwritable names and untyped props through a string record.
    assert!(
        jsx.contains("<details open>") || jsx.contains("<details\n"),
        "{jsx}"
    );
    assert!(jsx.contains("as Record<string, string>"), "{jsx}");
    assert!(
        jsx.contains("':class': 'x'") && jsx.contains("'@click': 'y'"),
        "{jsx}"
    );
    assert!(
        jsx.contains("defaultValue=\"x\""),
        "an input's captured value is its default: {jsx}"
    );
    assert!(
        !jsx.contains("tabIndex"),
        "a non-integer tabindex is not a number: {jsx}"
    );
    // No iframe content document; state-like classes stay on plain markup.
    assert!(!jsx.contains("inner"), "{jsx}");
    assert!(
        jsx.contains("state--active") && jsx.contains("is-layout-flex"),
        "{jsx}"
    );
}

#[test]
fn assembling_garden_and_microsoft_keeps_child_routes_apart_and_runs_nothing_from_the_originals() {
    for (dir, child_file, route_count) in [
        (
            "20261108-normalized/infinitegarden/capture",
            "articles_.commercial-intelligence-with-infinite-memory",
            3,
        ),
        (
            "20261108-microsoft-ai/capture-modules",
            "models_.mai-cyber-1-1-flash",
            3,
        ),
    ] {
        let root = Path::new(".mochi/trials").join(dir);
        if !root.join("manifest.json").exists() {
            eprintln!("skipping: {} not present", root.display());
            continue;
        }
        let project = mochi::semantic::project::assemble(
            &root,
            &mochi::semantic::project::Options {
                viewport: None,
                routes: None,
                tokens: false,
                styles: mochi::semantic::project::StyleSource::CapturedSheets,
            },
        )
        .unwrap();
        assert_eq!(project.report.routes.len(), route_count, "{dir}");
        assert!(
            project
                .files
                .contains_key(&format!("src/routes/{child_file}.tsx")),
            "{dir}: a captured child of a captured route must not nest under it: {:?}",
            project
                .files
                .keys()
                .filter(|k| k.starts_with("src/routes/"))
                .collect::<Vec<_>>()
        );
        for (path, bytes) in &project.files {
            let Ok(src) = std::str::from_utf8(bytes) else {
                continue;
            };
            if path.starts_with("src/pages/") || path.starts_with("src/components/") {
                assert!(
                    !src.contains("javascript:") && !src.contains("<script"),
                    "{dir} {path}"
                );
                assert!(!src.contains("next-route-announcer"), "{dir} {path}");
            }
            if path.starts_with("src/routes/") {
                assert!(
                    !src.contains("http-equiv") && !src.contains("httpEquiv"),
                    "{dir} {path}"
                );
                assert!(!src.contains("next-size-adjust"), "{dir} {path}");
            }
        }
        let gaps = project.report.gaps.join("\n");
        assert!(
            gaps.contains("inline `style`"),
            "{dir}: dropped inline styles must be a gap"
        );
    }
}

fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

// ---- real captures (skipped when the workspace evidence is absent) ----------------------

fn capture(dir: &str) -> Option<PageAnalysis> {
    let root = Path::new(".mochi/trials").join(dir);
    if !root.join("manifest.json").exists() {
        eprintln!("skipping: {} not present", root.display());
        return None;
    }
    Some(
        mochi::semantic::analyze_page(
            &root,
            Path::new("reconstruction/pages/page-00000-1440x900.json"),
            true,
        )
        .unwrap(),
    )
}

#[test]
fn natural_home_recognizes_its_dialog_tabs_toggle_group_accordion_and_hover_menu() {
    let Some(a) = capture("20261108-normalized/natural/capture") else {
        return;
    };
    let count = |p| {
        a.recognition
            .matches
            .iter()
            .filter(|m| m.primitive == p)
            .count()
    };
    assert_eq!(count(Primitive::Dialog), 1);
    assert_eq!(count(Primitive::Tabs), 2);
    assert_eq!(count(Primitive::ToggleGroup), 1);
    assert_eq!(count(Primitive::Accordion), 1);
    assert_eq!(count(Primitive::NavigationMenu), 1);
    let dialog = a
        .recognition
        .matches
        .iter()
        .find(|m| m.primitive == Primitive::Dialog)
        .unwrap();
    let js = dialog
        .js
        .as_ref()
        .expect("the nav state machine is recovered from the route chunk");
    assert!(
        js.app_code.iter().any(|c| c.contains("max-width: 1024px")),
        "{:?}",
        js.app_code
    );
    assert!(
        a.recognition.unrecognized.is_empty(),
        "{:?}",
        a.recognition.unrecognized
    );
}

#[test]
fn garden_and_microsoft_controls_are_recognized() {
    if let Some(a) = capture("20261108-normalized/infinitegarden/capture") {
        let acc = a
            .recognition
            .matches
            .iter()
            .find(|m| m.primitive == Primitive::Accordion)
            .unwrap();
        assert_eq!(acc.parts.iter().filter(|p| p.part == "Item").count(), 3);
        assert!(
            acc.gaps.iter().any(|g| g.contains("never closes")),
            "{:?}",
            acc.gaps
        );
    }
    if let Some(a) = capture("20261108-microsoft-ai/capture-modules") {
        let p: Vec<_> = a.recognition.matches.iter().map(|m| m.primitive).collect();
        for want in [
            Primitive::Tabs,
            Primitive::Switch,
            Primitive::Combobox,
            Primitive::Collapsible,
        ] {
            assert!(p.contains(&want), "{want:?} in {p:?}");
        }
    }
}
