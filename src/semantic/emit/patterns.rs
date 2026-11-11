//! One emitter per recognized pattern. Each returns the component source, an
//! optional typed data module, the `ui` primitives it imports and its gaps.
//! Nothing here invents content: text, links and keys come from the capture;
//! behavior comes from recovered state machines; what is neither stays a gap.
use super::jsx::Jsx;
use super::naming::component_name;
use super::ts::{ex_ts, ident, kebab, ts_string, type_name};
use super::ui::{self, wrapper_name};
use super::{EmitOptions, Emitted, EmittedComponent, PageBody, Skipped};
use crate::semantic::PageAnalysis;
use crate::semantic::ir::{Doc, NodeId};
use crate::semantic::js::{EffectFact, Ex, JsFacts, Lit, StateEffect, StateId};
use crate::semantic::patterns::{Links, Match, Part, Primitive, StateModel};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub fn supported(p: Primitive) -> bool {
    matches!(
        p,
        Primitive::Tabs
            | Primitive::ToggleGroup
            | Primitive::Toggle
            | Primitive::Accordion
            | Primitive::Collapsible
            | Primitive::Dialog
            | Primitive::Switch
            | Primitive::NavigationMenu
    )
}

#[derive(Default)]
struct Comp {
    source: String,
    data: Option<(String, String)>,
    ui: BTreeSet<Primitive>,
    gaps: Vec<String>,
    /// Props the component requires from its caller (name, TypeScript type).
    props: Vec<(String, String)>,
}

struct Cx<'a> {
    a: &'a PageAnalysis,
    doc: &'a Doc,
    links: Links,
    opts: &'a EmitOptions<'a>,
    names: BTreeMap<usize, String>,
    /// Instance root -> call of the extracted repeated component.
    group_calls: HashMap<NodeId, (String, String)>,
    state_classes: BTreeSet<String>,
    /// Nested match id -> nodes inside it whose click closes the enclosing dialog.
    nested_closers: HashMap<usize, Vec<NodeId>>,
    /// Dialog match id -> closing elements that cannot be wired (inside repeated-structure components).
    unwired_closers: HashMap<usize, Vec<NodeId>>,
    stats: std::rc::Rc<std::cell::RefCell<super::jsx::Stats>>,
    /// Route namespace: components live in `src/components/<ns>/`, data in `src/data/<ns>/`.
    ns: String,
}

impl<'a> Cx<'a> {
    fn comp_path(&self, name: &str) -> String {
        if self.ns.is_empty() {
            format!("src/components/{}.tsx", kebab(name))
        } else {
            format!("src/components/{}/{}.tsx", self.ns, kebab(name))
        }
    }
    fn comp_import(&self, name: &str) -> String {
        if self.ns.is_empty() {
            format!("@/components/{}", kebab(name))
        } else {
            format!("@/components/{}/{}", self.ns, kebab(name))
        }
    }
    fn data_path(&self, name: &str) -> String {
        if self.ns.is_empty() {
            format!("src/data/{}.ts", kebab(name))
        } else {
            format!("src/data/{}/{}.ts", self.ns, kebab(name))
        }
    }
    fn data_import(&self, name: &str) -> String {
        if self.ns.is_empty() {
            format!("@/data/{}", kebab(name))
        } else {
            format!("@/data/{}/{}", self.ns, kebab(name))
        }
    }

    fn jsx(&self) -> Jsx<'a> {
        let mut j = Jsx::new(self.doc, self.opts.style);
        j.overrides = self
            .group_calls
            .iter()
            .map(|(n, (_, call))| (*n, call.clone()))
            .collect();
        j.override_component = self
            .group_calls
            .iter()
            .map(|(n, (name, _))| (*n, name.clone()))
            .collect();
        j.stats = self.stats.clone();
        j.urls = self.opts.urls;
        // Class tokens the bundle adds or removes with state belong to the primitive's data attributes.
        j.state_classes = self.state_classes.clone();
        j
    }
}

fn parts<'m>(m: &'m Match, name: &str) -> Vec<&'m Part> {
    m.parts.iter().filter(|p| p.part == name).collect()
}

/// Text of a node when it holds nothing but text and inline wrappers.
fn plain_label(doc: &Doc, node: NodeId) -> Option<String> {
    let simple = doc.descendants(node).into_iter().all(|d| {
        let n = doc.node(d);
        n.text.is_some() || matches!(n.tag.as_str(), "span" | "b" | "i" | "em" | "strong" | "wbr")
    });
    let t = doc.visible_text(node, 200);
    (simple && !t.is_empty()).then_some(t)
}

fn element(tag: &str, attrs: &[String], children: Option<&str>, indent: usize) -> String {
    let pad = "  ".repeat(indent);
    let open = if attrs.is_empty() {
        format!("<{tag}")
    } else if attrs.iter().map(String::len).sum::<usize>() + tag.len() < 72 {
        format!("<{tag} {}", attrs.join(" "))
    } else {
        format!(
            "<{tag}\n{}{pad}",
            attrs
                .iter()
                .map(|a| format!("{pad}  {a}\n"))
                .collect::<String>()
        )
    };
    match children {
        None => format!("{pad}{open} />\n"),
        Some(c) if c.trim().is_empty() => format!("{pad}{open} />\n"),
        Some(c) => format!("{pad}{open}>\n{c}{pad}</{tag}>\n"),
    }
}

/// Attributes the primitive owns; everything else on a part is carried over.
const OWNED: &[&str] = &[
    "role",
    "type",
    "id",
    "tabindex",
    "aria-checked",
    "aria-modal",
    "aria-orientation",
    "aria-haspopup",
    "aria-labelledby",
    "aria-selected",
    "aria-expanded",
    "aria-pressed",
    "aria-controls",
    "inert",
    "hidden",
    "href",
    "target",
    "rel",
];

fn part_attrs(jsx: &Jsx<'_>, node: NodeId, extra: &[String]) -> Vec<String> {
    let mut v = jsx.part_attributes(node, OWNED);
    v.extend(extra.iter().cloned());
    v
}

fn import_line(p: Primitive, names: &BTreeSet<String>) -> String {
    format!(
        "import {{ {} }} from '@/components/ui/{}'\n",
        names.iter().cloned().collect::<Vec<_>>().join(", "),
        p.ui_file()
    )
}

fn w(p: Primitive, part: &str) -> String {
    wrapper_name(p, part)
}

fn indent_block(s: &str, levels: usize) -> String {
    let pad = "  ".repeat(levels);
    s.lines()
        .map(|l| {
            if l.is_empty() {
                "\n".to_owned()
            } else {
                format!("{pad}{l}\n")
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------

fn tabs(cx: &Cx<'_>, m: &Match, name: &str) -> Result<Comp, String> {
    let doc = cx.doc;
    let tabs = parts(m, "Tab");
    if tabs.is_empty() {
        return Err("no Tab parts".into());
    }
    let jsx = cx.jsx();
    let mut gaps = m.gaps.clone();
    let list = parts(m, "List")[0].node;
    let root = parts(m, "Root")[0].node;
    let mut items = String::new();
    let mut union = Vec::new();
    for t in &tabs {
        let key = t.key.clone().ok_or("tab without key")?;
        let label = plain_label(doc, t.node).unwrap_or_else(|| {
            gaps.push(format!(
                "tab `{key}` has non-text content; its data row keeps the text only, the list keeps the captured markup"
            ));
            doc.visible_text(t.node, 200)
        });
        let href = (t.render_as.as_deref() == Some("a"))
            .then(|| doc.node(t.node).attr("href").unwrap_or("").to_owned());
        items.push_str(&format!(
            "  {{ value: {}, label: {}{} }},\n",
            ts_string(&key),
            ts_string(&label),
            href.map(|h| format!(", href: {}", ts_string(&h)))
                .unwrap_or_default()
        ));
        union.push(key);
    }
    let has_href = tabs.iter().any(|t| t.render_as.as_deref() == Some("a"));
    let id = ident(name);
    let ty = type_name(name);
    let data_file = cx.data_path(name);
    let data = format!(
        "// Tab values and labels read from the capture; `value` keys come from {}.\nexport const {id}Items = [\n{items}] as const\n\nexport type {ty}Value = (typeof {id}Items)[number]['value']\n",
        tabs[0].key_source.as_deref().unwrap_or("the capture")
    );
    // Styles are read per captured element, so the tab that was selected carries the selected
    // look: every tab takes the class of the first one that was not.
    let selected_key = m.props.get("defaultValue").cloned();
    let template_tab = tabs
        .iter()
        .find(|t| t.key.is_some() && t.key != selected_key)
        .unwrap_or(&tabs[0]);
    let tab_class = jsx.class_value(template_tab.node);
    let tab_attrs: Vec<String> = {
        let mut a = vec!["key={tab.value}".to_owned(), "value={tab.value}".to_owned()];
        if !tab_class.is_empty() {
            a.push(format!("className={}", super::jsx::attr_value(&tab_class)));
        }
        if has_href {
            a.push("nativeButton={false}".to_owned());
            a.push("render={<a href={tab.href} />}".to_owned());
        }
        a
    };
    let tab_panels = parts(m, "Panel");
    let all_panels = tab_panels.len() == tabs.len();
    let differing_tab_classes = tabs
        .iter()
        .filter(|t| t.key != selected_key)
        .any(|t| jsx.class_value(t.node) != tab_class);
    if differing_tab_classes {
        gaps.push("tabs carry different class lists after dropping state classes; the first unselected tab's classes are used".into());
    }
    if let Some(sel) = tabs
        .iter()
        .find(|t| t.key.is_some() && t.key == selected_key)
        && jsx.class_value(sel.node) != tab_class
    {
        gaps.push("the selected tab's captured styles differ from the others'; the selected look is not emitted (no state variant is derived from a capture of one state)".into());
    }
    let controlled = matches!(m.state, StateModel::Controlled { .. });
    let default = m.props.get("defaultValue").cloned();
    let list_class = jsx.class_value(list);
    let root_class = if root == list {
        String::new()
    } else {
        jsx.class_value(root)
    };
    let list_attrs = {
        let mut a: Vec<String> = keep_tag(doc, list, "div").into_iter().collect();
        if !list_class.is_empty() {
            a.push(format!("className={}", super::jsx::attr_value(&list_class)));
        }
        if let Some(l) = doc.node(list).attr("aria-label") {
            a.push(format!("aria-label={}", super::jsx::attr_value(l)));
        }
        a
    };
    // One captured element holds the tabs: the list keeps its box (and the tablist role), and the
    // `Root` Base UI needs around it generates none (`display: contents`), so the parent's layout
    // sees the same child as in the capture.
    let mut root_attrs: Vec<String> = if root == list {
        vec!["className=\"contents\"".to_owned()]
    } else {
        keep_tag(doc, root, "div").into_iter().collect()
    };
    if !root_class.is_empty() {
        root_attrs.push(format!("className={}", super::jsx::attr_value(&root_class)));
    }
    let (props_ty, sig, root_extra) = if controlled {
        (
            format!(
                "type Props = Omit<ComponentProps<typeof {r}>, 'children' | 'value' | 'defaultValue' | 'onValueChange'> & {{\n  value: {ty}Value\n  onValueChange: (value: {ty}Value) => void\n{extra}}}",
                r = w(Primitive::Tabs, "Root"),
                extra = if all_panels {
                    ""
                } else {
                    "  children?: ReactNode\n"
                }
            ),
            format!(
                "{{ value, onValueChange, {}...props }}",
                if all_panels { "" } else { "children, " }
            ),
            vec![
                "value={value}".to_owned(),
                "onValueChange={(next) => onValueChange(next as ".to_owned() + &ty + "Value)}",
            ],
        )
    } else {
        (
            format!(
                "type Props = Omit<ComponentProps<typeof {r}>, 'children'>{extra}",
                r = w(Primitive::Tabs, "Root"),
                extra = if all_panels {
                    ""
                } else {
                    " & { children?: ReactNode }"
                }
            ),
            format!(
                "{{ defaultValue = {}, {}...props }}",
                ts_string(&default.clone().unwrap_or_else(|| union[0].clone())),
                if all_panels { "" } else { "children, " }
            ),
            vec!["defaultValue={defaultValue}".to_owned()],
        )
    };
    let mut root_all = root_attrs;
    root_all.extend(root_extra);
    root_all.push("{...props}".to_owned());
    // A tab that holds more than text (an icon, a wrapper) cannot be a `{ value, label }` row:
    // each tab is then written out with the children the capture shows.
    let rich_tabs = tabs.iter().any(|t| {
        doc.descendants(t.node)
            .into_iter()
            .any(|d| doc.node(d).is_element())
    });
    let list_el = if rich_tabs {
        let mut written = String::new();
        for t in &tabs {
            let key = t.key.clone().ok_or("tab without key")?;
            let mut attrs = vec![format!("value={}", ts_string(&key))];
            if !tab_class.is_empty() {
                attrs.push(format!("className={}", super::jsx::attr_value(&tab_class)));
            }
            if t.render_as.as_deref() == Some("a") {
                attrs.push("nativeButton={false}".to_owned());
                attrs.push(format!(
                    "render={{<a href={} />}}",
                    super::jsx::attr_value(doc.node(t.node).attr("href").unwrap_or(""))
                ));
            }
            written.push_str(&element(
                &w(Primitive::Tabs, "Tab"),
                &attrs,
                Some(&jsx.children(t.node, 2)),
                1,
            ));
        }
        element(&w(Primitive::Tabs, "List"), &list_attrs, Some(&written), 0)
    } else {
        let list_children = element(
            &w(Primitive::Tabs, "Tab"),
            &tab_attrs,
            Some("      {tab.label}\n"),
            3,
        );
        element(
            &w(Primitive::Tabs, "List"),
            &list_attrs,
            Some(&format!(
                "  {{{id}Items.map((tab) => (\n{list_children}  ))}}\n"
            )),
            0,
        )
    };
    let mut tab_imports: Vec<&str> = vec!["TabsList", "TabsRoot", "TabsTab"];
    // The list and the panels replace their nodes; everything else in the root stays in place.
    let mut body_jsx = cx.jsx();
    body_jsx.overrides.insert(list, list_el.clone());
    if all_panels {
        tab_imports.push("TabsPanel");
        for p in &tab_panels {
            let key = p.key.clone().ok_or("tab panel without key")?;
            let mut pattrs = vec![format!("value={}", ts_string(&key))];
            pattrs.extend(keep_tag(doc, p.node, "div"));
            pattrs.extend(part_attrs(&jsx, p.node, &[]));
            // Panels the capture holds while their tab is not selected stay mounted.
            if doc.inert(p.node) || doc.hidden(p.node) {
                pattrs.push("keepMounted".into());
            }
            body_jsx.overrides.insert(
                p.node,
                element(
                    &w(Primitive::Tabs, "Panel"),
                    &pattrs,
                    Some(&jsx.children(p.node, 1)),
                    0,
                ),
            );
        }
    } else {
        gaps.push(format!(
            "{} of {} tabs have no tabpanel in the DOM/ARIA: panel content is supplied by the caller as `children`",
            tabs.len() - tab_panels.len(),
            tabs.len()
        ));
    }
    let mut inner = if root == list {
        indent_block(&list_el, 3)
    } else {
        body_jsx.children(root, 3)
    };
    if !all_panels {
        inner.push_str("      {children}\n");
    }
    let source = format!(
        "import type {{ ComponentProps{rn} }} from 'react'\n{imports}import {{ {items_import}type {ty}Value }} from '{file}'\n\n{props_ty}\n\nexport function {name}({sig}: Props) {{\n  return (\n{root}  )\n}}\n",
        rn = if all_panels { "" } else { ", ReactNode" },
        imports = import_line(
            Primitive::Tabs,
            &tab_imports.iter().map(|s| s.to_string()).collect()
        ),
        items_import = if rich_tabs {
            String::new()
        } else {
            format!("{id}Items, ")
        },
        file = cx.data_import(name),
        root = element(&w(Primitive::Tabs, "Root"), &root_all, Some(&inner), 2),
    );
    Ok(Comp {
        source,
        data: Some((data_file, data)),
        ui: [Primitive::Tabs].into(),
        gaps,
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------

fn toggle_group(cx: &Cx<'_>, m: &Match, name: &str) -> Result<Comp, String> {
    let _doc = cx.doc;
    let jsx = cx.jsx();
    let items = parts(m, "Toggle");
    if items.is_empty() {
        return Err("no Toggle parts".into());
    }
    let root = parts(m, "")[0].node;
    let controlled = matches!(m.state, StateModel::Controlled { .. });
    let default = m
        .props
        .get("defaultValue")
        .cloned()
        .unwrap_or_else(|| "[]".into());
    // Toggles replace their nodes; anything else inside the group stays in place.
    let mut group_jsx = cx.jsx();
    for t in &items {
        let key = t.key.clone().ok_or("toggle without key")?;
        let mut attrs = vec![format!("value={}", ts_string(&key))];
        attrs.extend(part_attrs(&jsx, t.node, &[]));
        group_jsx.overrides.insert(
            t.node,
            element(
                &w(Primitive::Toggle, ""),
                &attrs,
                Some(&jsx.children(t.node, 1)),
                0,
            ),
        );
    }
    let body = group_jsx.children(root, 3);
    let mut root_attrs: Vec<String> = keep_tag(cx.doc, root, "div").into_iter().collect();
    root_attrs.extend(part_attrs(&jsx, root, &[]));
    let ty = type_name(name);
    let keys: Vec<String> = items.iter().filter_map(|t| t.key.clone()).collect();
    // The caller may control it with value/onValueChange; the captured selection is the default.
    let multiple = m.props.get("multiple").map(String::as_str) == Some("true");
    root_attrs.push(format!("multiple={{{multiple}}}"));
    root_attrs.push("defaultValue={defaultValue}".to_string());
    root_attrs.push("{...props}".into());
    let props_ty = format!(
        "type Props = Omit<ComponentProps<typeof {g}>, 'children' | 'multiple'>",
        g = w(Primitive::ToggleGroup, "")
    );
    let sig = format!("{{ defaultValue = {default}, ...props }}");
    let _ = controlled;
    let id = ident(name);
    let source = format!(
        "import type {{ ComponentProps }} from 'react'\n{ig}{it}\nexport const {id}Values = [{vals}] as const\nexport type {ty}Value = (typeof {id}Values)[number]\n\n{props_ty}\n\n/** {selection} */\nexport function {name}({sig}: Props) {{\n  return (\n{root}  )\n}}\n",
        ig = import_line(
            Primitive::ToggleGroup,
            &[w(Primitive::ToggleGroup, "")].into_iter().collect()
        ),
        it = import_line(
            Primitive::Toggle,
            &[w(Primitive::Toggle, "")].into_iter().collect()
        ),
        vals = keys
            .iter()
            .map(|k| ts_string(k))
            .collect::<Vec<_>>()
            .join(", "),
        selection = if multiple {
            "Multiple selection (the original had several pressed buttons)."
        } else {
            "Single selection (the original had exactly one pressed button)."
        },
        root = element(&w(Primitive::ToggleGroup, ""), &root_attrs, Some(&body), 2),
    );
    let mut gaps = m.gaps.clone();
    gaps.push("the state used by the original to switch the visible content is not part of this component; bind it through value/onValueChange".into());
    Ok(Comp {
        source,
        data: None,
        ui: [Primitive::ToggleGroup, Primitive::Toggle].into(),
        gaps,
        ..Default::default()
    })
}

fn toggle(cx: &Cx<'_>, m: &Match, name: &str) -> Result<Comp, String> {
    let _doc = cx.doc;
    let jsx = cx.jsx();
    let node = parts(m, "")[0].node;
    let attrs = {
        let mut a = part_attrs(&jsx, node, &[]);
        a.push(format!(
            "defaultPressed={{{}}}",
            m.props
                .get("defaultPressed")
                .cloned()
                .unwrap_or_else(|| "false".into())
        ));
        a.push("{...props}".into());
        a
    };
    let source = format!(
        "import type {{ ComponentProps }} from 'react'\n{imp}\nexport function {name}(props: ComponentProps<typeof {t}>) {{\n  return (\n{root}  )\n}}\n",
        imp = import_line(
            Primitive::Toggle,
            &[w(Primitive::Toggle, "")].into_iter().collect()
        ),
        t = w(Primitive::Toggle, ""),
        root = element(
            &w(Primitive::Toggle, ""),
            &attrs,
            Some(&jsx.children(node, 3)),
            2
        ),
    );
    Ok(Comp {
        source,
        data: None,
        ui: [Primitive::Toggle].into(),
        gaps: m.gaps.clone(),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------

fn disclosure(
    cx: &Cx<'_>,
    m: &Match,
    name: &str,
    dialog_links: &HashMap<NodeId, Vec<String>>,
) -> Result<Comp, String> {
    let doc = cx.doc;
    let mut jsx = cx.jsx();
    for (n, a) in dialog_links {
        jsx.extra_attrs.insert(*n, a.clone());
    }
    // Links in here close the enclosing dialog in the original: the dialog hands in `onNavigate`.
    let navigate = m.primitive == Primitive::Accordion && cx.nested_closers.contains_key(&m.id);
    if navigate {
        for n in &cx.nested_closers[&m.id] {
            jsx.extra_attrs
                .insert(*n, vec!["onClick={onNavigate}".to_owned()]);
        }
    }
    let accordion = m.primitive == Primitive::Accordion;
    let p = m.primitive;
    let triggers = parts(m, "Trigger");
    let panels = parts(m, "Panel");
    if triggers.is_empty() || triggers.len() != panels.len() {
        return Err("unequal triggers and panels".into());
    }
    let keep = m.props.contains_key("keepMounted");
    let root = parts(m, "Root")[0].node;
    let mut gaps = m.gaps.clone();
    let id = ident(name);
    let ty = type_name(name);
    let controlled = matches!(m.state, StateModel::Controlled { .. });
    let mut body = String::new();
    let mut keys = Vec::new();
    let mut uses_header = false;
    if accordion {
        let items = parts(m, "Item");
        let mut item_src: HashMap<NodeId, String> = HashMap::new();
        for (i, item) in items.iter().enumerate() {
            let key = item.key.clone().ok_or("accordion item without key")?;
            keys.push(key.clone());
            let mut iattrs = vec![format!("value={}", ts_string(&key))];
            if let Some(tag) = &item.render_as {
                iattrs.push(format!("render={{<{tag} />}}"));
            }
            iattrs.extend(part_attrs(&jsx, item.node, &[]));
            let t = triggers[i];
            let mut tattrs = part_attrs(&jsx, t.node, &[]);
            tattrs.sort();
            let trigger_el = element(&w(p, "Trigger"), &tattrs, Some(&jsx.children(t.node, 1)), 0);
            let header = parts(m, "Header")
                .into_iter()
                .find(|h| doc.is_ancestor(h.node, t.node));
            // The trigger (inside its header, when there is one) and the panel replace their
            // nodes; the rest of the item (a wrapper div, a visible link beside the button) stays.
            match header {
                Some(h) => {
                    uses_header = true;
                    let header_el = element(
                        &w(p, "Header"),
                        &[format!(
                            "render={{<{} />}}",
                            h.render_as.clone().unwrap_or_else(|| "h3".into())
                        )],
                        Some(&indent_block(&trigger_el, 1)),
                        0,
                    );
                    jsx.overrides.insert(h.node, header_el);
                }
                None => {
                    jsx.overrides.insert(t.node, trigger_el);
                }
            }
            let mut pattrs: Vec<String> =
                keep_tag(doc, panels[i].node, "div").into_iter().collect();
            pattrs.extend(part_attrs(&jsx, panels[i].node, &[]));
            if keep {
                pattrs.push("keepMounted".into());
            }
            let panel_el = element(
                &w(p, "Panel"),
                &pattrs,
                Some(&jsx.children(panels[i].node, 1)),
                0,
            );
            jsx.overrides.insert(panels[i].node, panel_el);
            item_src.insert(
                item.node,
                element(&w(p, "Item"), &iattrs, Some(&jsx.children(item.node, 4)), 3),
            );
        }
        // Children of the root that are not items (links beside the accordion) stay in place.
        for &c in &doc.node(root).children {
            match item_src.remove(&c) {
                Some(src) => body.push_str(&src),
                None => body.push_str(&jsx.render(c, 3)),
            }
        }
        if !uses_header {
            gaps.push("the original has no heading around its triggers: Accordion.Header is omitted; runtime behavior of Accordion.Trigger without a Header is not exercised here".into());
        }
    } else {
        let t = triggers[0];
        let mut tattrs = part_attrs(&jsx, t.node, &[]);
        tattrs.sort();
        let mut pattrs: Vec<String> = keep_tag(doc, panels[0].node, "div").into_iter().collect();
        pattrs.extend(part_attrs(&jsx, panels[0].node, &[]));
        if keep {
            pattrs.push("keepMounted".into());
        }
        let trigger_el = element(&w(p, "Trigger"), &tattrs, Some(&jsx.children(t.node, 1)), 0);
        if root != t.node && doc.is_ancestor(root, panels[0].node) {
            // Trigger and panel replace their nodes; the rest of the root stays in place.
            let panel_el = element(
                &w(p, "Panel"),
                &pattrs,
                Some(&jsx.children(panels[0].node, 1)),
                0,
            );
            jsx.overrides.insert(t.node, trigger_el);
            jsx.overrides.insert(panels[0].node, panel_el);
            body.push_str(&jsx.children(root, 3));
        } else {
            body.push_str(&indent_block(&trigger_el, 3));
            body.push_str(&element(
                &w(p, "Panel"),
                &pattrs,
                Some("      {children}\n"),
                3,
            ));
            gaps.push("the panel is not inside this component's root in the DOM (trigger and panel share only a distant ancestor): its content is supplied as `children`, and Collapsible.Root cannot wrap both where the original placed them".into());
        }
    }
    // A root that is the trigger element itself must not lend its attributes to Root.
    let mut root_attrs = if triggers.iter().any(|t| t.node == root) {
        Vec::new()
    } else {
        let mut a: Vec<String> = keep_tag(doc, root, "div").into_iter().collect();
        a.extend(part_attrs(&jsx, root, &[]));
        a
    };

    let source = if accordion {
        let default = m.props.get("defaultValue").cloned();
        let multiple = m
            .props
            .get("multiple")
            .cloned()
            .unwrap_or_else(|| "false".into());
        root_attrs.push(format!("multiple={{{multiple}}}"));

        // Controlled by the caller when value/onValueChange are passed; the captured
        // expansion is the default either way.
        root_attrs.push("defaultValue={defaultValue}".into());
        root_attrs.push("{...props}".into());
        let sig = format!(
            "{{ defaultValue = {}, {}...props }}",
            default
                .map(|d| d.replace('"', "'"))
                .unwrap_or_else(|| "[]".into()),
            if navigate { "onNavigate, " } else { "" }
        );
        let _ = controlled;
        format!(
            "import type {{ ComponentProps }} from 'react'\n{imp}\nexport const {id}Items = [{keys}] as const\nexport type {ty}Key = (typeof {id}Items)[number]\n\ntype Props = Omit<ComponentProps<typeof {r}>, 'children' | 'multiple'>{nav_prop}\n\nexport function {name}({sig}: Props) {{\n  return (\n{root}  )\n}}\n",
            imp = import_line(
                p,
                &[
                    "AccordionItem",
                    "AccordionPanel",
                    "AccordionRoot",
                    "AccordionTrigger"
                ]
                .iter()
                .map(|s| s.to_string())
                .chain(uses_header.then(|| "AccordionHeader".to_string()))
                .collect()
            ),
            keys = keys
                .iter()
                .map(|k| ts_string(k))
                .collect::<Vec<_>>()
                .join(", "),
            r = w(p, "Root"),
            nav_prop = if navigate {
                " & { onNavigate?: () => void }"
            } else {
                ""
            },
            root = element(&w(p, "Root"), &root_attrs, Some(&body), 2),
        )
    } else {
        let open = m
            .props
            .get("defaultOpen")
            .cloned()
            .unwrap_or_else(|| "false".into());
        root_attrs.push(format!("defaultOpen={{{open}}}"));
        root_attrs.push("{...props}".into());
        format!(
            "import type {{ ComponentProps, ReactNode }} from 'react'\n{imp}\ntype Props = Omit<ComponentProps<typeof {r}>, 'children' | 'defaultOpen'> & {{ children?: ReactNode }}\n\nexport function {name}({{ children, ...props }}: Props) {{\n  return (\n{root}  )\n}}\n",
            imp = import_line(
                p,
                &["CollapsibleRoot", "CollapsibleTrigger", "CollapsiblePanel"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            ),
            r = w(p, "Root"),
            root = element(&w(p, "Root"), &root_attrs, Some(&body), 2),
        )
    };
    Ok(Comp {
        source,
        data: None,
        ui: [p].into(),
        gaps,
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------

fn switch(cx: &Cx<'_>, m: &Match, name: &str) -> Result<Comp, String> {
    let jsx = cx.jsx();
    let root = parts(m, "Root")[0].node;
    let mut attrs = part_attrs(&jsx, root, &[]);
    attrs.push(format!(
        "defaultChecked={{{}}}",
        m.props
            .get("defaultChecked")
            .cloned()
            .unwrap_or_else(|| "false".into())
    ));
    attrs.push("{...props}".into());
    let thumb = element(&w(Primitive::Switch, "Thumb"), &[], None, 3);
    let source = format!(
        "import type {{ ComponentProps }} from 'react'\n{imp}\nexport function {name}(props: ComponentProps<typeof SwitchRoot>) {{\n  return (\n{root}  )\n}}\n",
        imp = import_line(
            Primitive::Switch,
            &["SwitchRoot", "SwitchThumb"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        ),
        root = element("SwitchRoot", &attrs, Some(&thumb), 2),
    );
    Ok(Comp {
        source,
        data: None,
        ui: [Primitive::Switch].into(),
        gaps: m.gaps.clone(),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------

/// `render={<tag />}` when the captured element is not the tag the Base UI part renders, so the
/// composition keeps the captured element instead of adding another beside it.
fn keep_tag(doc: &Doc, node: NodeId, default: &str) -> Option<String> {
    let tag = doc.node(node).tag.as_str();
    (tag != default && !tag.is_empty() && !matches!(tag, "html" | "body" | "head"))
        .then(|| format!("render={{<{tag} />}}"))
}

fn hover_navigation(cx: &Cx<'_>, m: &Match, name: &str) -> Result<Comp, String> {
    let doc = cx.doc;
    let mut jsx = cx.jsx();
    let items = parts(m, "Item");
    let triggers = parts(m, "Trigger");
    if items.is_empty() || items.len() != triggers.len() {
        return Err("hover navigation without matching items and triggers".into());
    }
    let mut gaps = m.gaps.clone();
    let keys: Vec<String> = items.iter().filter_map(|i| i.key.clone()).collect();
    let ty = type_name(name);
    let id = ident(name);
    let root = parts(m, "Root")[0].node;
    let list = parts(m, "List").first().map(|p| p.node);
    let mut body = String::new();
    let mut item_src: HashMap<NodeId, String> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        let key = &keys[i];
        let mut tattrs = part_attrs(&jsx, triggers[i].node, &[]);
        tattrs.sort();
        if doc.node(triggers[i].node).role() == Some("link") {
            gaps.push(format!(
                "trigger `{key}` is a link in the original; its destination is not carried by NavigationMenu.Trigger"
            ));
        }
        let trigger_children = match plain_label(doc, triggers[i].node) {
            Some(l) => format!("  {l}\n"),
            None => jsx.children(triggers[i].node, 1),
        };
        let mut pair = element(
            &w(Primitive::NavigationMenu, "Trigger"),
            &tattrs,
            Some(&trigger_children),
            0,
        );
        pair.push_str(&element(
            &w(Primitive::NavigationMenu, "Content"),
            &[],
            Some(&format!(
                "  {{dropdowns.{}}}\n",
                if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    key.clone()
                } else {
                    format!("[{}]", ts_string(key))
                }
            )),
            0,
        ));
        // The trigger and its dropdown replace the trigger node; the rest of the item stays.
        jsx.overrides.insert(triggers[i].node, pair);
        let mut iattrs = vec![format!("value={}", ts_string(key))];
        iattrs.extend(keep_tag(doc, item.node, "li"));
        iattrs.extend(part_attrs(&jsx, item.node, &[]));
        item_src.insert(
            item.node,
            element(
                &w(Primitive::NavigationMenu, "Item"),
                &iattrs,
                Some(&jsx.children(item.node, 4)),
                3,
            ),
        );
    }
    // Other children of the list stay in place; a bare link becomes an item holding a Link.
    let container = list.unwrap_or(root);
    let mut uses_link = false;
    for &c in &doc.node(container).children {
        if let Some(src) = item_src.remove(&c) {
            body.push_str(&src);
        } else if doc.node(c).is_element() && doc.node(c).role() == Some("link") {
            uses_link = true;
            let attrs = jsx.part_attributes(c, &["role", "type", "id", "tabindex"]);
            let link = element(
                &w(Primitive::NavigationMenu, "Link"),
                &attrs,
                Some(&jsx.children(c, 1)),
                0,
            );
            // `NavigationMenu.Link` needs no `Item`: a wrapper would be an element the capture lacks.
            body.push_str(&indent_block(&link, 3));
        } else {
            body.push_str(&jsx.render(c, 3));
        }
    }
    let popup = {
        let viewport = element(&w(Primitive::NavigationMenu, "Viewport"), &[], None, 5);
        let popup = element(
            &w(Primitive::NavigationMenu, "Popup"),
            &[],
            Some(&viewport),
            4,
        );
        let positioner = element(
            &w(Primitive::NavigationMenu, "Positioner"),
            &[],
            Some(&popup),
            3,
        );
        element(
            &w(Primitive::NavigationMenu, "Portal"),
            &[],
            Some(&positioner),
            2,
        )
    };
    let mut list_attrs: Vec<String> = Vec::new();
    let split = list.filter(|l| *l != root);
    match split {
        Some(l) => {
            list_attrs.extend(keep_tag(doc, l, "ul"));
            list_attrs.extend(part_attrs(&jsx, l, &[]));
        }
        // One captured element holds the items: List keeps it (box, classes, attributes) and the
        // `Root` Base UI needs around it generates no box, so the parent's layout and the items'
        // flex/grid context are the capture's. Root renders a `div`, not a nested `nav`.
        None => {
            list_attrs.extend(keep_tag(doc, root, "ul"));
            list_attrs.extend(part_attrs(&jsx, root, &[]));
        }
    }
    let list_el = element(
        &w(Primitive::NavigationMenu, "List"),
        &list_attrs,
        Some(&indent_block(&body, 1)),
        3,
    );
    let mut root_attrs: Vec<String> = match split {
        Some(_) => {
            let mut a: Vec<String> = keep_tag(doc, root, "nav").into_iter().collect();
            a.extend(part_attrs(&jsx, root, &[]));
            a
        }
        None => vec!["render={<div />}".into(), "className=\"contents\"".into()],
    };
    root_attrs.push("{...props}".into());
    let mut inner = list_el;
    inner.push_str(&popup);
    let source = format!(
        "import type {{ ComponentProps, ReactNode }} from 'react'\n{imp}\nexport const {id}Items = [{keys}] as const\nexport type {ty}Key = (typeof {id}Items)[number]\n\ntype Props = Omit<ComponentProps<typeof NavigationMenuRoot>, 'children'> & {{\n  /** Dropdown body per item: rendered by script in the original and absent from the capture. */\n  dropdowns: Record<{ty}Key, ReactNode>\n}}\n\nexport function {name}({{ dropdowns, ...props }}: Props) {{\n  return (\n{root}  )\n}}\n",
        imp = import_line(
            Primitive::NavigationMenu,
            &[
                "NavigationMenuContent",
                "NavigationMenuItem",
                "NavigationMenuList",
                if uses_link {
                    "NavigationMenuLink"
                } else {
                    "NavigationMenuList"
                },
                "NavigationMenuPopup",
                "NavigationMenuPortal",
                "NavigationMenuPositioner",
                "NavigationMenuRoot",
                "NavigationMenuTrigger",
                "NavigationMenuViewport"
            ]
            .iter()
            .map(|s| s.to_string())
            .collect()
        ),
        keys = keys
            .iter()
            .map(|k| ts_string(k))
            .collect::<Vec<_>>()
            .join(", "),
        root = element("NavigationMenuRoot", &root_attrs, Some(&inner), 2),
    );
    Ok(Comp {
        source,
        data: None,
        ui: [Primitive::NavigationMenu].into(),
        gaps,
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------

/// TypeScript statements for state effects, with state names; `None` when an effect is not expressible.
fn effect_stmt(e: &StateEffect, names: &HashMap<StateId, (String, String)>) -> Option<String> {
    let (_, setter) = names.get(&e.state())?;
    Some(match e {
        StateEffect::Set { value, .. } => {
            let plain: HashMap<StateId, String> =
                names.iter().map(|(k, v)| (*k, v.0.clone())).collect();
            format!("{setter}({})", ex_ts(value, &plain)?)
        }
        StateEffect::Toggle { .. } => {
            let (value, _) = names.get(&e.state())?;
            format!("{setter}(!{value})")
        }
        _ => return None,
    })
}

/// Elements inside the dialog whose click handler closes it, with that handler's effects.
fn dialog_closers(cx: &Cx<'_>, m: &Match) -> Vec<(NodeId, Vec<StateEffect>)> {
    let (Some(js), Some(popup)) = (m.js.as_ref(), parts(m, "Popup").first().map(|p| p.node)) else {
        return Vec::new();
    };
    let Some(state) = js.state else {
        return Vec::new();
    };
    let mut out: Vec<(NodeId, Vec<StateEffect>)> = Vec::new();
    for (node, l) in cx.links.all() {
        if !cx.doc.is_ancestor(popup, node) || cx.a.js[l.file].file != js.file {
            continue;
        }
        let site = &cx.a.js[l.file].sites[l.site];
        for h in site.handlers.iter().filter(|h| h.event == "onClick") {
            let closes = h.effects.iter().any(|e| {
                matches!(e, StateEffect::Set { state: s, value: Ex::Lit { value: Lit::Bool(false) } } if *s == state)
            });
            if closes && !out.iter().any(|(n, _)| *n == node) {
                out.push((node, h.effects.clone()));
            }
        }
    }
    out
}

fn dialog(
    cx: &Cx<'_>,
    m: &Match,
    name: &str,
    nested: &[&Match],
) -> Result<(Comp, Vec<String>), String> {
    let doc = cx.doc;
    let jsx_probe = cx.jsx();
    let js =
        m.js.as_ref()
            .ok_or("dialog without recovered state: the open state cannot be modeled")?;
    let state = js.state.ok_or("dialog without state id")?;
    let file_ix =
        cx.a.js
            .iter()
            .position(|f| f.file == js.file)
            .ok_or("bundle file not loaded")?;
    let f: &JsFacts = &cx.a.js[file_ix];
    let popup = parts(m, "Popup")[0].node;
    let triggers = parts(m, "Trigger");
    let trigger = triggers.first().ok_or("dialog without a trigger")?.node;
    let mut gaps = m.gaps.clone();
    // State names in the emitted component.
    let mut names: HashMap<StateId, (String, String)> = HashMap::new();
    names.insert(state, ("open".into(), "setOpen".into()));
    let mut states_code = String::from("  const [open, setOpen] = useState(false)\n");
    let mut nested_use = String::new();
    let mut nested_imports = String::new();
    let mut overrides: HashMap<NodeId, String> = HashMap::new();
    let mut resets: Vec<String> = Vec::new();
    let mut accordion_types = Vec::new();
    let mut pending_calls: Vec<(usize, NodeId, String, String, String, String)> = Vec::new();
    for n in nested {
        let Some(njs) = &n.js else { continue };
        let Some(ns) = njs.state else { continue };
        if njs.file != js.file || f.state(ns).function != f.state(state).function {
            continue;
        }
        let comp = cx
            .names
            .get(&n.id)
            .ok_or("nested pattern without a component")?;
        let (var, setter) = match n.primitive {
            Primitive::Accordion => ("expandedItem".to_owned(), "setExpandedItem".to_owned()),
            _ => continue,
        };
        let item_ty = format!("{}Key", type_name(comp));
        names.insert(ns, (var.clone(), setter.clone()));
        states_code.push_str(&format!(
            "  const [{var}, {setter}] = useState<{item_ty} | null>(null)\n"
        ));
        resets.push(format!("{setter}(null)"));
        nested_imports.push_str(&format!(
            "import {{ {comp}, type {item_ty} }} from '{}'\n",
            cx.comp_import(comp)
        ));
        accordion_types.push(item_ty.clone());
        pending_calls.push((n.id, n.root, comp.clone(), var, setter, item_ty));
        let _ = &mut nested_use;
    }
    let mut jsx = cx.jsx();
    let closers = dialog_closers(cx, m);
    let nested_roots: Vec<NodeId> = pending_calls.iter().map(|c| c.1).collect();
    for (nid, root_node, comp, var, setter, item_ty) in &pending_calls {
        let navigate = match cx
            .nested_closers
            .get(nid)
            .and_then(|nodes| closers.iter().find(|(n, _)| nodes.contains(n)))
        {
            Some((_, effects)) => {
                let stmts: Option<Vec<String>> =
                    effects.iter().map(|e| effect_stmt(e, &names)).collect();
                match stmts {
                    Some(st) => format!("\n  onNavigate={{() => {{ {} }}}}", st.join("; ")),
                    None => {
                        gaps.push(format!("links inside `{comp}` close the dialog with effects that are not expressible"));
                        String::new()
                    }
                }
            }
            None => String::new(),
        };
        overrides.insert(
            *root_node,
            format!(
                "<{comp}\n  value={{{var} ? [{var}] : []}}\n  onValueChange={{(value) => {setter}((value[0] as {item_ty} | undefined) ?? null)}}{navigate}\n/>"
            ),
        );
    }
    // These calls carry props and import the component themselves.
    for r in &nested_roots {
        jsx.override_component.remove(r);
    }
    jsx.overrides.extend(overrides);
    // Handlers that close the dialog from inside it (links). Inside a nested component they
    // arrive through its `onNavigate` prop; inside a repeated-structure component they cannot.
    for (node, effects) in &closers {
        if nested_roots
            .iter()
            .any(|r| *r == *node || doc.is_ancestor(*r, *node))
        {
            continue;
        }
        if cx.group_calls.contains_key(node)
            || doc
                .ancestors(*node)
                .any(|a| cx.group_calls.contains_key(&a))
        {
            gaps.push(format!(
                "node {node} closes the dialog on click, but it renders inside a repeated-structure component that has no click prop: the close is not wired"
            ));
            continue;
        }
        let stmts: Option<Vec<String>> = effects.iter().map(|e| effect_stmt(e, &names)).collect();
        match stmts {
            Some(st) => {
                jsx.extra_attrs.insert(
                    *node,
                    vec![format!("onClick={{() => {{ {} }}}}", st.join("; "))],
                );
            }
            None => gaps.push(format!(
                "an onClick on node {node} closes the dialog with effects that are not expressible from the recovered summary"
            )),
        }
    }
    // Effects the primitive does not provide.
    let mut effects_code = String::new();
    let mut props: Vec<(String, String)> = Vec::new();
    let mut imports_react = vec!["useState".to_owned()];
    for rec in &f.effects {
        if !rec.chain.contains(&f.state(state).function) {
            continue;
        }
        match &rec.fact {
            EffectFact::MediaQuery {
                query,
                when_matches,
                effects,
            } if effects.iter().any(|e| e.state() == state) => {
                let stmts: Option<Vec<String>> =
                    effects.iter().map(|e| effect_stmt(e, &names)).collect();
                let Some(stmts) = stmts else {
                    gaps.push(format!("media query `{query}` effect not expressible"));
                    continue;
                };
                let cond = match when_matches {
                    Some(true) => "event.matches",
                    Some(false) => "!event.matches",
                    None => "true",
                };
                imports_react.push("useEffect".into());
                effects_code.push_str(&format!(
                    "  useEffect(() => {{\n    const query = window.matchMedia({q})\n    const onChange = (event: MediaQueryListEvent) => {{\n      if ({cond}) {{ {s} }}\n    }}\n    query.addEventListener('change', onChange)\n    return () => query.removeEventListener('change', onChange)\n  }}, [])\n\n",
                    q = ts_string(query),
                    s = stmts.join("; ")
                ));
            }
            EffectFact::DepsChange {
                deps,
                effects,
                unconditional,
            } if effects.iter().any(|e| e.state() == state) => {
                if !*unconditional {
                    gaps.push(format!(
                        "an effect on [{}] resets state only under a condition in the original; it is not emitted",
                        deps.join(", ")
                    ));
                    continue;
                }
                let stmts: Option<Vec<String>> =
                    effects.iter().map(|e| effect_stmt(e, &names)).collect();
                let Some(stmts) = stmts else {
                    gaps.push("a dependency effect is not expressible".into());
                    continue;
                };
                let dep = deps.first().cloned().unwrap_or_default();
                let valid =
                    !dep.is_empty() && dep.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !valid || deps.len() != 1 {
                    gaps.push(format!(
                        "effect on [{}] not emitted: dependency names are not identifiers",
                        deps.join(", ")
                    ));
                    continue;
                }
                props.push((dep.clone(), "string".into()));
                imports_react.push("useEffect".into());
                effects_code.push_str(&format!(
                    "  // The original resets whenever `{dep}` changes (a member destructured from a hook call in the bundle); the caller supplies it.\n  useEffect(() => {{\n    {s}\n  }}, [{dep}])\n\n",
                    s = stmts.join("\n    ")
                ));
            }
            _ => {}
        }
    }
    imports_react.sort();
    imports_react.dedup();
    // Trigger label from the original binding.
    let mut plain_names: HashMap<StateId, String> = HashMap::new();
    for (k, v) in &names {
        plain_names.insert(*k, v.0.clone());
    }
    let mut trigger_attrs = part_attrs(&jsx_probe, trigger, &[]);
    let bound_label = cx
        .links
        .site(&cx.a.js, trigger)
        .and_then(|(_, site)| site.bindings.iter().find(|(k, _)| k == "aria-label"));
    if let Some((_, ex)) = bound_label {
        // The label follows the open state in the original.
        match ex_ts(ex, &plain_names) {
            Some(e) => {
                trigger_attrs.retain(|a| !a.starts_with("aria-label="));
                trigger_attrs.push(format!("aria-label={{{e}}}"));
            }
            None => gaps.push(
                "trigger aria-label binding is not expressible; the captured label is kept".into(),
            ),
        }
    }
    let mounted_inert = doc.node(popup).has_attr("inert") && !doc.node(popup).has_attr("hidden");
    if mounted_inert {
        gaps.push("the popup is mounted while closed (keepMounted, `inert`, not `hidden`) in a portal: Base UI wraps it in one extra `div` under <body> that the capture does not have; its styles are the closed-state ones baked from the capture, so the open look is not generated".into());
    }
    gaps.push("trigger children (the icon) are emitted as captured in the closed state; whether the bundle renders different children when open is not analyzed".into());
    if let Some(unwired) = cx.unwired_closers.get(&m.id) {
        gaps.push(format!(
            "{} element(s) inside nested components close the dialog on click in the original but render through repeated-structure components that take no click prop: not wired",
            unwired.len()
        ));
    }
    // Bundle sites that close the dialog but have no captured element to attach to.
    for (si, site) in f.sites.iter().enumerate() {
        let closes = site.handlers.iter().any(|h| {
            h.event == "onClick"
                && h.effects.iter().any(|e| {
                    matches!(e, StateEffect::Set { state: s, value: Ex::Lit { value: Lit::Bool(false) } } if *s == state)
                })
        });
        let linked_in_popup = cx
            .links
            .all()
            .any(|(node, l)| l.file == file_ix && l.site == si && doc.is_ancestor(popup, node));
        if closes && !linked_in_popup {
            gaps.push(format!(
                "bundle site <{}> (line {}) closes the dialog on click, but no captured element is linked to it: not wired",
                site.tag, site.line
            ));
        }
    }
    let mut popup_attrs = part_attrs(&jsx_probe, popup, &[]);
    if mounted_inert {
        // Written below as `inert={!open}`: the captured bare `inert` would be a duplicate.
        popup_attrs.retain(|a| a != "inert");
    }
    // Mounted while closed, as the capture has it: present in the DOM, `inert` instead of
    // Base UI's `hidden` (the captured classes decide how it looks).
    if mounted_inert {
        popup_attrs.push("hidden={false}".into());
        popup_attrs.push("inert={!open}".into());
    }
    let popup_el = element(
        &w(Primitive::Dialog, "Popup"),
        &popup_attrs,
        Some(&jsx.children(popup, 6)),
        4,
    );
    let portal = element(
        &w(Primitive::Dialog, "Portal"),
        &["keepMounted".into()],
        Some(&popup_el),
        3,
    );
    let trigger_el = element(
        &w(Primitive::Dialog, "Trigger"),
        &trigger_attrs,
        Some(&jsx.children(trigger, 4)),
        3,
    );
    let reset_stmt = if resets.is_empty() {
        String::new()
    } else {
        format!("\n      if (!next) {{ {} }}", resets.join("; "))
    };
    let root_el = element(
        &w(Primitive::Dialog, "Root"),
        &[
            "open={open}".into(),
            format!("onOpenChange={{(next) => {{\n      setOpen(next){reset_stmt}\n    }}}}"),
            format!(
                "modal={{{}}}",
                m.props
                    .get("modal")
                    .cloned()
                    .unwrap_or_else(|| "true".into())
            ),
        ],
        Some(&format!("{trigger_el}{portal}")),
        2,
    );
    let props_decl = if props.is_empty() {
        String::new()
    } else {
        format!(
            "type Props = {{ {} }}\n\n",
            props
                .iter()
                .map(|(n, t)| format!("{n}: {t}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    };
    let sig = if props.is_empty() {
        String::new()
    } else {
        format!(
            "{{ {} }}: Props",
            props
                .iter()
                .map(|p| p.0.clone())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let source = format!(
        "import {{ {react} }} from 'react'\n{imp}{nested_imports}\n{props_decl}export function {name}({sig}) {{\n{states_code}\n{effects_code}  return (\n{root_el}  )\n}}\n",
        react = imports_react.join(", "),
        imp = import_line(
            Primitive::Dialog,
            &["DialogPopup", "DialogPortal", "DialogRoot", "DialogTrigger"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        ),
    );
    let provided = js.provided_by_primitive.clone();
    gaps.push(format!(
        "provided by Dialog and therefore not re-implemented: {}",
        provided.join("; ")
    ));
    Ok((
        Comp {
            source,
            data: None,
            ui: [Primitive::Dialog].into(),
            gaps,
            props: props.clone(),
        },
        accordion_types,
    ))
}

// ---------------------------------------------------------------------------

/// Add imports for the components a file rendered (recorded while rendering, not found by
/// searching the generated text).
fn add_imports(cx: &Cx<'_>, source: &str, used: &BTreeSet<String>, own: &str) -> String {
    let imports: String = used
        .iter()
        .filter(|n| n.as_str() != own)
        .map(|n| format!("import {{ {n} }} from '{}'\n", cx.comp_import(n)))
        .collect();
    if imports.is_empty() {
        return source.to_owned();
    }
    let mut lines: Vec<&str> = source.lines().collect();
    let at = lines
        .iter()
        .rposition(|l| l.starts_with("import "))
        .map(|i| i + 1)
        .unwrap_or_else(|| lines.iter().take_while(|l| l.starts_with("//")).count());
    let tail = lines.split_off(at).join("\n");
    let head = lines.join("\n");
    format!(
        "{head}\n{imports}{}{tail}\n",
        if at == 0 { "\n" } else { "" }
    )
}

/// A pattern component that was emitted: match id, component name, props it requires.
type PlaceInfo = (usize, String, Vec<(String, String)>);

/// Do corresponding elements of the instances carry different classes (after the style bridge)
/// or different inline styles? Those are not props of the component, so grouping them would give
/// every instance the first one's look and position.
fn styles_differ(cx: &Cx<'_>, instances: &[NodeId]) -> Option<String> {
    let doc = cx.doc;
    let jsx = cx.jsx();
    let mut level: Vec<Vec<NodeId>> = vec![instances.to_vec()];
    while let Some(nodes) = level.pop() {
        let first = nodes[0];
        if doc.node(first).is_element() {
            let class = jsx.class_value(first);
            let style = doc.node(first).attr("style");
            if nodes[1..]
                .iter()
                .any(|n| jsx.class_value(*n) != class || doc.node(*n).attr("style") != style)
            {
                return Some(format!(
                    "the <{}> at the same place carries different classes or inline styles in the instances",
                    doc.node(first).tag
                ));
            }
        }
        let width = doc.node(first).children.len();
        if nodes.iter().any(|n| doc.node(*n).children.len() != width) {
            return None;
        }
        for i in 0..width {
            level.push(nodes.iter().map(|n| doc.node(*n).children[i]).collect());
        }
    }
    None
}

/// Components for repeated structures whose name and every varying slot are semantic.
fn group_components(cx: &mut Cx<'_>, taken: &BTreeSet<String>, out: &mut Emitted) {
    let doc = cx.doc;
    let a = cx.a;
    // Pattern parts are rendered by the pattern emitters, never as group instances.
    let part_nodes: BTreeSet<NodeId> = a
        .recognition
        .matches
        .iter()
        .flat_map(|m| {
            m.parts
                .iter()
                .map(|p| p.node)
                .chain(std::iter::once(m.root))
        })
        .collect();
    let mut used: BTreeSet<String> = taken.clone();
    struct Pending {
        name: String,
        group: usize,
    }
    let mut pending: Vec<Pending> = Vec::new();
    for (gi, g) in a.components.groups.iter().enumerate() {
        if !g.extractable {
            continue;
        }
        if g.slots.is_empty() {
            out.skipped_groups.push(format!(
                "`{}` ({} identical instances, first node {}): no varying slot, so a component would have no props",
                g.component_name.as_deref().unwrap_or("?"),
                g.instances.len(),
                g.instances[0]
            ));
            continue;
        }
        if g.instances.iter().any(|i| {
            part_nodes.contains(i) || doc.descendants(*i).iter().any(|d| part_nodes.contains(d))
        }) {
            out.skipped_groups.push(format!(
                "`{}` ({} instances, first node {}): instances contain or are parts of a recognized pattern, which the pattern emitters render",
                g.component_name.as_deref().unwrap_or("?"),
                g.instances.len(),
                g.instances[0]
            ));
            continue;
        }
        if let Some(why) = styles_differ(cx, &g.instances) {
            out.skipped_groups.push(format!(
                "`{}` ({} instances, first node {}): {why}; a component renders every instance from the first one, so they stay plain markup",
                g.component_name.as_deref().unwrap_or("?"),
                g.instances.len(),
                g.instances[0]
            ));
            continue;
        }
        let name = g.component_name.clone().unwrap();
        if ui::is_wrapper_name(&name) || !used.insert(name.clone()) {
            out.skipped_groups.push(format!(
                "`{name}` ({} instances, first node {}): the name is already used by another component or a ui wrapper",
                g.instances.len(),
                g.instances[0]
            ));
            continue;
        }
        pending.push(Pending { name, group: gi });
    }
    // Calls first (the bodies render nested groups through them).
    for p in &pending {
        let g = &a.components.groups[p.group];
        let mapper = cx.jsx();
        for (ix, inst) in g.instances.iter().enumerate() {
            let mut props = Vec::new();
            for s in &g.slots {
                let n = s.name.clone().unwrap();
                // A prop that carries an asset URL goes through the same mapping as inline markup,
                // so the file is copied and the reference points at it.
                let v = &match (s.kind, s.attr.as_deref()) {
                    (crate::semantic::components::SlotKind::Attribute, Some(attr)) => {
                        let el = crate::semantic::components::element_at(doc, *inst, &s.path);
                        mapper.rewrite_url(doc.node(el).tag.as_str(), attr, &s.values[ix])
                    }
                    _ => s.values[ix].clone(),
                };
                props.push(if s.ts_type == "number" {
                    format!("{n}={{{v}}}")
                } else {
                    format!("{n}={}", super::jsx::attr_value(v))
                });
            }
            cx.group_calls.insert(
                *inst,
                (
                    p.name.clone(),
                    format!("<{} {} />", p.name, props.join(" ")),
                ),
            );
        }
    }
    for p in &pending {
        let g = &a.components.groups[p.group];
        let mut jsx = cx.jsx();
        jsx.overrides.remove(&g.instances[0]);
        jsx.override_component.remove(&g.instances[0]);
        let first = g.instances[0];
        let mut fields = Vec::new();
        let mut names = Vec::new();
        for s in &g.slots {
            let n = s.name.clone().unwrap();
            let at = crate::semantic::components::element_at(doc, first, &s.path);
            match (s.kind, s.attr.as_deref()) {
                (crate::semantic::components::SlotKind::Text, _) => {
                    jsx.text_slots.insert(at, format!("{{{n}}}"));
                }
                (_, Some(attr)) => {
                    if let Some(js) = super::jsx::attr_name(attr) {
                        jsx.attr_slots
                            .insert((at, attr.to_owned()), format!("{js}={{{n}}}"));
                    }
                }
                _ => {}
            }
            fields.push(format!("  {n}: {}", s.ts_type));
            names.push(n);
        }
        let body = jsx.render(first, 2);
        let used = std::mem::take(&mut cx.stats.borrow_mut().used_components);
        let source = format!(
            "// Generated by `mochi semantic`: {} identical-structure instances of `{}`; props are the positions where their text or attributes differ.
export interface {n}Props {{
{fields}
}}

export function {n}({{ {names} }}: {n}Props) {{
  return (
{body}  )
}}
",
            g.instances.len(),
            g.name_source.as_deref().unwrap_or(""),
            n = p.name,
            fields = fields.join("\n").replace("\\n", "\n"),
            names = names.join(", "),
        );
        let file = cx.comp_path(&p.name);
        let source = add_imports(cx, &source, &used, &p.name);
        out.files.insert(file.clone(), source);
        out.group_components.push(super::GroupComponent {
            name: p.name.clone(),
            file,
            instances: g.instances.clone(),
            props: g.slots.iter().filter_map(|s| s.name.clone()).collect(),
            name_source: g.name_source.clone().unwrap_or_default(),
        });
    }
}

/// Render the page body: seeded compositions and repeated components are already overrides;
/// compositions that need props from the page (controlled tabs, the dialog, hover navigation)
/// get their calls and hooks here. What cannot be placed is listed with the reason.
fn render_page(
    cx: &Cx<'_>,
    infos: &[PlaceInfo],
    also: &BTreeMap<usize, Vec<usize>>,
    out: &Emitted,
) -> PageBody {
    let doc = cx.doc;
    let mut page = PageBody::default();
    let mut jsx = cx.jsx();
    let mut hooks: Vec<String> = Vec::new();
    let mut imports: BTreeSet<String> = BTreeSet::new();
    // Drop what earlier renders recorded: only this render counts toward coverage and imports.
    cx.stats.borrow_mut().used_components.clear();
    cx.stats.borrow_mut().used_nodes.clear();
    cx.stats.borrow_mut().elements_dropped.clear();
    // Decide every placement first, then commit shallow before deep, so a component is never
    // half placed (its popup removed while its trigger stays) and order cannot change the result.
    struct Candidate {
        mid: usize,
        name: String,
        node: NodeId,
        call: String,
        hooks: Vec<String>,
        imports: Vec<String>,
        removes: Vec<NodeId>,
        duplicates: Vec<usize>,
    }
    let mut cands: Vec<Candidate> = Vec::new();
    for (mid, name, props) in infos {
        let m = &cx.a.recognition.matches[*mid];
        let ids: Vec<usize> = std::iter::once(*mid)
            .chain(also.get(mid).cloned().unwrap_or_default())
            .collect();
        let controlled = matches!(m.state, StateModel::Controlled { .. });
        let needs_props = match m.primitive {
            Primitive::Tabs => controlled,
            Primitive::Dialog | Primitive::NavigationMenu => true,
            _ => false,
        };
        if !needs_props {
            continue;
        }
        // Components that need page state are placed once; later usages stay plain markup.
        let target = &cx.a.recognition.matches[ids[0]];
        let mut c = Candidate {
            mid: *mid,
            name: name.clone(),
            node: 0,
            call: String::new(),
            hooks: Vec::new(),
            imports: Vec::new(),
            removes: Vec::new(),
            duplicates: ids[1..].to_vec(),
        };
        match m.primitive {
            Primitive::Tabs => {
                let Some(root) = parts(target, "Root").first().map(|p| p.node) else {
                    continue;
                };
                let var = ident(&format!("{name} value"));
                let setter = ident(&format!("set {name} value"));
                let ty = format!("{}Value", type_name(name));
                let default = target
                    .props
                    .get("defaultValue")
                    .cloned()
                    .unwrap_or_default();
                c.hooks.push(format!(
                    "const [{var}, {setter}] = useState<{ty}>({})",
                    ts_string(&default)
                ));
                c.imports.push("import { useState } from 'react'".into());
                c.imports.push(format!(
                    "import type {{ {ty} }} from '{}'",
                    cx.data_import(name)
                ));
                c.node = root;
                c.call = format!("<{name} value={{{var}}} onValueChange={{{setter}}} />");
            }
            Primitive::Dialog => {
                let Some(trigger) = parts(target, "Trigger").first().map(|p| p.node) else {
                    continue;
                };
                let mut args = String::new();
                let mut unsupported = false;
                for (prop, _) in props {
                    if prop == "pathname" {
                        c.hooks.push("const pathname = useRouterState({ select: (state) => state.location.pathname })".into());
                        c.imports
                            .push("import { useRouterState } from '@tanstack/react-router'".into());
                        args.push_str(" pathname={pathname}");
                    } else {
                        page.unplaced.push((
                            *mid,
                            format!(
                                "`{name}` requires prop `{prop}`, which the page cannot supply"
                            ),
                        ));
                        unsupported = true;
                    }
                }
                if unsupported {
                    continue;
                }
                // The popup lives elsewhere in the captured DOM (a portal): the component renders it.
                c.removes.push(parts(target, "Popup")[0].node);
                c.node = trigger;
                c.call = format!("<{name}{args} />");
            }
            Primitive::NavigationMenu => {
                let Some(root) = parts(target, "Root").first().map(|p| p.node) else {
                    continue;
                };
                let keys: Vec<String> = parts(target, "Item")
                    .iter()
                    .filter_map(|p| p.key.clone())
                    .collect();
                let map = keys
                    .iter()
                    .map(|k| {
                        if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                            && !k.starts_with(|c: char| c.is_ascii_digit())
                        {
                            format!("{k}: null")
                        } else {
                            format!("{}: null", ts_string(k))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                c.node = root;
                c.call = format!("<{name} dropdowns={{{{ {map} }}}} />");
            }
            _ => continue,
        }
        cands.push(c);
    }
    let depth = |n: NodeId| doc.ancestors(n).count();
    cands.sort_by_key(|c| (depth(c.node), c.mid));
    let mut placed_nodes: Vec<NodeId> = Vec::new();
    for c in cands {
        let covered = |n: NodeId| {
            doc.ancestors(n)
                .any(|a| jsx.overrides.contains_key(&a) || placed_nodes.contains(&a))
        };
        if covered(c.node) || c.removes.iter().any(|r| covered(*r)) {
            page.unplaced.push((
                c.mid,
                format!(
                    "`{}` sits inside another component's content, which renders it",
                    c.name
                ),
            ));
            continue;
        }
        placed_nodes.push(c.node);
        for r in &c.removes {
            jsx.overrides.insert(*r, String::new());
        }
        for h in &c.hooks {
            if !hooks.contains(h) {
                hooks.push(h.clone());
            }
        }
        imports.extend(c.imports.iter().cloned());
        jsx.overrides.insert(c.node, c.call.clone());
        jsx.override_component.insert(c.node, c.name.clone());
        for other in &c.duplicates {
            page.unplaced.push((
                *other,
                format!(
                    "`{}` is used again at node {}; only the first usage is placed (it needs page state)",
                    c.name,
                    cx.a.recognition.matches[*other].root
                ),
            ));
        }
    }
    let Some(body) = doc.nodes.iter().find(|n| n.tag == "body").map(|n| n.id) else {
        page.unplaced.push((0, "the capture has no <body>".into()));
        return page;
    };
    page.jsx = jsx.children(body, 3);
    let used = std::mem::take(&mut cx.stats.borrow_mut().used_components);
    let used_nodes = std::mem::take(&mut cx.stats.borrow_mut().used_nodes);
    page.elements_dropped = std::mem::take(&mut cx.stats.borrow_mut().elements_dropped);
    for n in &used {
        imports.insert(format!("import {{ {n} }} from '{}'", cx.comp_import(n)));
    }
    page.placed = used.into_iter().collect();
    page.hooks = hooks;
    page.imports = imports.into_iter().collect();
    let in_body: Vec<NodeId> = doc
        .descendants(body)
        .into_iter()
        .filter(|d| doc.node(*d).is_element())
        .collect();
    let mut covered: BTreeSet<NodeId> = BTreeSet::new();
    for n in used_nodes {
        covered.insert(n);
        covered.extend(doc.descendants(n));
    }
    page.covered_nodes = in_body
        .iter()
        .copied()
        .filter(|d| covered.contains(d))
        .collect();
    page.elements = in_body.len();
    page.covered_elements = in_body.iter().filter(|d| covered.contains(d)).count();
    page.raw_elements = page.elements - page.covered_elements;
    page.scripts_omitted = in_body
        .iter()
        .filter(|d| {
            matches!(
                doc.node(**d).tag.as_str(),
                "script" | "style" | "noscript" | "template"
            )
        })
        .count();
    let _ = out;
    page
}

pub fn emit(a: &PageAnalysis, opts: &EmitOptions<'_>) -> Emitted {
    let doc = &a.doc;
    let mut out = Emitted::default();
    let mut cx = Cx {
        a,
        doc,
        links: Links::build(doc, &a.js),
        opts,
        names: BTreeMap::new(),
        group_calls: HashMap::new(),
        state_classes: a
            .js
            .iter()
            .flat_map(|f| f.sites.iter())
            .flat_map(|s| s.class_conditional.iter().map(|(t, _)| t.clone()))
            .collect(),
        stats: Default::default(),
        nested_closers: HashMap::new(),
        unwired_closers: HashMap::new(),
        ns: opts.namespace.to_owned(),
    };
    // Names; duplicate usage of the same component is emitted once.
    let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
    let mut also: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut emittable: Vec<&Match> = Vec::new();
    let mut name_sources: BTreeMap<usize, String> = BTreeMap::new();
    for m in &a.recognition.matches {
        if !supported(m.primitive) {
            continue;
        }
        if m.primitive == Primitive::NavigationMenu && m.rule != "navigation-menu.hover-state" {
            out.skipped.push(Skipped {
                match_id: m.id,
                primitive: m.primitive,
                reason: "popup-links navigation: the structure around the triggers is not preserved by the emitter yet".into(),
            });
            continue;
        }
        let named = component_name(doc, m).and_then(|(name, src)| {
            if !ui::is_wrapper_name(&name) {
                return Some((name, src));
            }
            // The name is a ui wrapper's: qualify it with the nearest named ancestor.
            let anc = doc.ancestors(m.root).find_map(|a| {
                let n = doc.node(a);
                n.attr("data-component").map(str::to_owned).or_else(|| {
                    n.classes
                        .iter()
                        .find(|c| {
                            c.len() >= 3
                                && !crate::semantic::components::is_state_class(c)
                                && !crate::semantic::components::is_utility_class(c)
                        })
                        .cloned()
                })
            })?;
            let qualified = super::ts::join_words(&[&anc, &name]);
            (!ui::is_wrapper_name(&qualified))
                .then(|| (qualified, format!("{src}, qualified by ancestor `{anc}`")))
        });
        match named {
            Some((name, source)) => {
                name_sources.insert(m.id, source);
                if let Some(first) = by_name.get(&name) {
                    // Same name and primitive: only identical usage may share a component.
                    let a_m = &a.recognition.matches[*first];
                    // Identical usage: the same parts with the same keys and visible text.
                    let sig = |x: &Match| -> Vec<(String, Option<String>, String)> {
                        x.parts
                            .iter()
                            .map(|p| (p.part.clone(), p.key.clone(), doc.visible_text(p.node, 80)))
                            .collect()
                    };
                    if a_m.primitive == m.primitive && a_m.label == m.label && sig(a_m) == sig(m) {
                        also.entry(*first).or_default().push(m.id);
                        continue;
                    }
                    out.skipped.push(Skipped {
                        match_id: m.id,
                        primitive: m.primitive,
                        reason: format!("name `{name}` is already used by a different pattern; no further semantic name is derivable"),
                    });
                    continue;
                }
                by_name.insert(name.clone(), m.id);
                cx.names.insert(m.id, name);
                emittable.push(m);
            }
            None => out.skipped.push(Skipped {
                match_id: m.id,
                primitive: m.primitive,
                reason: "no semantic name: no data-component, class block or accessible name on the pattern".into(),
            }),
        }
    }
    for m in &a.recognition.matches {
        if !supported(m.primitive) && !out.skipped.iter().any(|s| s.match_id == m.id) {
            out.skipped.push(Skipped {
                match_id: m.id,
                primitive: m.primitive,
                reason: "no composition emitter for this primitive yet (recognized and reported; no ui wrapper or component is generated)"
                    .into(),
            });
        }
    }
    let taken: BTreeSet<String> = cx.names.values().cloned().collect();
    group_components(&mut cx, &taken, &mut out);
    // A pattern that needs no props can be rendered by name wherever another component's
    // content contains it (a tab set inside an accordion panel).
    for m in &emittable {
        let controlled = matches!(m.state, StateModel::Controlled { .. });
        let seed = match m.primitive {
            Primitive::Tabs => !controlled,
            Primitive::ToggleGroup
            | Primitive::Toggle
            | Primitive::Accordion
            | Primitive::Switch => true,
            Primitive::Collapsible => parts(m, "Trigger").iter().all(|t| t.node != m.root),
            _ => false,
        };
        let root_node = parts(m, "Root")
            .first()
            .or(parts(m, "").first())
            .map(|p| p.node);
        let is_other_part = a
            .recognition
            .matches
            .iter()
            .any(|o| o.id != m.id && o.parts.iter().any(|p| Some(p.node) == root_node));
        if let (true, Some(node)) = (seed && !is_other_part, root_node) {
            let name = cx.names[&m.id].clone();
            cx.group_calls
                .entry(node)
                .or_insert((name.clone(), format!("<{name} />")));
            // Identical usages elsewhere on the page render the same component.
            for dup in also.get(&m.id).into_iter().flatten() {
                let dup_root = parts(&a.recognition.matches[*dup], "Root")
                    .first()
                    .or(parts(&a.recognition.matches[*dup], "").first())
                    .map(|p| p.node);
                if let Some(n) = dup_root {
                    cx.group_calls
                        .entry(n)
                        .or_insert((name.clone(), format!("<{name} />")));
                }
            }
        }
    }
    // Nested patterns (an accordion inside a dialog).
    let mut nested_of: BTreeMap<usize, Vec<&Match>> = BTreeMap::new();
    for d in emittable
        .iter()
        .filter(|m| m.primitive == Primitive::Dialog)
    {
        let popup = parts(d, "Popup")[0].node;
        for n in emittable.iter().filter(|n| n.id != d.id) {
            if doc.is_ancestor(popup, n.root) {
                nested_of.entry(d.id).or_default().push(n);
            }
        }
    }
    let also_all = also.clone();
    let mut infos: Vec<PlaceInfo> = Vec::new();
    let nested_ids: BTreeSet<usize> = nested_of.values().flatten().map(|m| m.id).collect();
    for d in emittable
        .iter()
        .filter(|m| m.primitive == Primitive::Dialog)
    {
        let closers = dialog_closers(&cx, d);
        for n in nested_of.get(&d.id).map(Vec::as_slice).unwrap_or(&[]) {
            let inside: Vec<NodeId> = closers
                .iter()
                .map(|(node, _)| *node)
                .filter(|node| *node == n.root || doc.is_ancestor(n.root, *node))
                .collect();
            // A closer inside a repeated-structure component has no click prop to carry it.
            let (unwired, wired): (Vec<NodeId>, Vec<NodeId>) =
                inside.into_iter().partition(|node| {
                    doc.ancestors(*node)
                        .take_while(|a| *a != n.root)
                        .chain(std::iter::once(*node))
                        .any(|a| {
                            a != n.root
                                && cx
                                    .group_calls
                                    .get(&a)
                                    .is_some_and(|(c, _)| !cx.names.values().any(|v| v == c))
                        })
                });
            if !wired.is_empty() {
                cx.nested_closers.insert(n.id, wired);
            }
            if !unwired.is_empty() {
                cx.unwired_closers.entry(d.id).or_default().extend(unwired);
            }
        }
    }
    for m in &emittable {
        let name = cx.names[&m.id].clone();
        let name_source = name_sources[&m.id].clone();
        let result = match m.primitive {
            Primitive::Tabs => tabs(&cx, m, &name),
            Primitive::ToggleGroup => toggle_group(&cx, m, &name),
            Primitive::Toggle => toggle(&cx, m, &name),
            Primitive::Accordion | Primitive::Collapsible => {
                disclosure(&cx, m, &name, &HashMap::new())
            }
            Primitive::Switch => switch(&cx, m, &name),
            Primitive::NavigationMenu => hover_navigation(&cx, m, &name),
            Primitive::Dialog => dialog(
                &cx,
                m,
                &name,
                nested_of.get(&m.id).map(Vec::as_slice).unwrap_or(&[]),
            )
            .map(|(c, _)| c),
            _ => Err("unsupported".into()),
        };
        match result {
            Ok(c) => {
                let file = cx.comp_path(&name);
                let mut source = format!(
                    "// Generated by `mochi semantic` from rule `{}` (confidence {:.2}).\n// Evidence and gaps: see the semantic report. Original class names are kept until the styles workstream replaces them.\n{}",
                    m.rule, m.confidence, c.source
                );
                if !source.ends_with('\n') {
                    source.push('\n');
                }
                let used = std::mem::take(&mut cx.stats.borrow_mut().used_components);
                let source = add_imports(&cx, &source, &used, &name);
                out.files.insert(file.clone(), source);
                let data_file = c.data.map(|(path, text)| {
                    out.files.insert(path.clone(), text);
                    path
                });
                out.ui_primitives.extend(c.ui.iter().copied());
                infos.push((m.id, name.clone(), c.props.clone()));
                out.components.push(EmittedComponent {
                    match_id: m.id,
                    primitive: m.primitive,
                    name,
                    name_source,
                    file,
                    data_file,
                    also_matches: also.remove(&m.id).unwrap_or_default(),
                    gaps: c.gaps,
                });
            }
            Err(reason) => out.skipped.push(Skipped {
                match_id: m.id,
                primitive: m.primitive,
                reason,
            }),
        }
    }
    let _ = nested_ids;
    if opts.page {
        out.page = Some(render_page(&cx, &infos, &also_all, &out));
    }
    {
        let st = cx.stats.borrow();
        out.inline_style_attributes_dropped = st.inline_style_attributes_dropped;
        out.state_classes_dropped = st.state_classes_dropped.clone();
        out.svg_subtrees = st.svg_subtrees;
        out.custom_attributes = st.custom_attributes;
        if !st.custom_elements.is_empty() {
            let decls: String = st
                .custom_elements
                .iter()
                .map(|t| format!("      '{t}': React.DetailedHTMLProps<React.HTMLAttributes<HTMLElement>, HTMLElement>\n"))
                .collect();
            out.files.insert(
                "src/types/custom-elements.d.ts".into(),
                format!("// Custom elements rendered by the captured page, declared so TSX accepts them.\nimport type React from 'react'\n\ndeclare module 'react' {{\n  namespace JSX {{\n    interface IntrinsicElements {{\n{decls}    }}\n  }}\n}}\n"),
            );
            out.custom_elements = st.custom_elements.clone();
        }
    }
    for p in out.ui_primitives.clone() {
        if let Some((path, text)) = ui::file(p) {
            out.files.insert(path, text);
        }
    }
    out
}
