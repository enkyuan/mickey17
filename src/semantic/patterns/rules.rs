//! The rule table. Each rule names the Base UI primitive, how candidate roots
//! are found, and a list of weighted checks over the accessibility tree,
//! computed style and recovered state machines. Required checks gate the match;
//! supporting checks raise the confidence; contra checks lower it. The binders in
//! `matcher.rs` then assign parts; they never override a rule's verdict.
use super::{EvidenceSource, Primitive};
use crate::semantic::ir::{Doc, NodeId};
use crate::semantic::js::JsFacts;
use crate::semantic::patterns::Links;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    Required,
    Supporting,
    Contra,
}

/// How candidate roots are enumerated.
#[derive(Debug, Clone, Copy)]
pub enum RootSel {
    /// Elements whose effective role is one of these.
    Role(&'static [&'static str]),
    /// Groups of `aria-expanded` triggers with their panels (accordion / collapsible).
    Disclosure,
    /// Sibling buttons carrying `aria-pressed`.
    PressedGroup,
    /// Elements with `aria-haspopup` (menu, popover, navigation-menu triggers).
    PopupTrigger,
    /// Native radios grouped by `name`.
    RadioNames,
    /// Native form fields: input, textarea, select.
    FormField,
    /// `button` elements and `role=button` elements.
    ButtonLike,
    /// Wrappers whose pointer-enter handler sets one state to distinct keys (hover menus).
    HoverMenu,
}

/// A candidate: a root plus the members the selector found for it.
#[derive(Debug, Clone, Default)]
pub struct Candidate {
    pub root: NodeId,
    pub members: Vec<NodeId>,
    /// Panel per member for disclosure groups (`None` when unresolved).
    pub panels: Vec<Option<NodeId>>,
    /// How each panel was found: `aria-controls`, `labelledby`, `adjacent-sibling`.
    pub panel_via: Vec<&'static str>,
    /// Item keys recovered from the bundle (hover menus).
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum Pred {
    RoleIs(&'static str),
    TagIs(&'static str),
    AriaTrue(&'static str),
    AttrPresent(&'static str),
    /// At least / at most this many members.
    Members {
        min: usize,
        max: usize,
    },
    /// Exactly one member selected (`aria-selected` or `aria-pressed`).
    OneSelected,
    /// Every member's `aria-controls` resolves to an element.
    MembersControlPanels,
    /// Every member has a resolved panel (any method).
    PanelsResolved,
    /// Every panel is referenced through `aria-controls` / `aria-labelledby`.
    PanelsLinkedByAria,
    /// Panels are removed from interaction while collapsed (`inert`, `hidden`, hidden style).
    PanelsClosedAreInert,
    /// `position: fixed` (or absolute with a z-index) on the root.
    Layered,
    /// `inert`/`hidden`/`aria-hidden` on the root in the captured (initial) state.
    InitiallyClosed,
    /// A recovered JSX site is linked to the root or a member.
    JsBound,
    /// The root or first member has an accessible name.
    Named,
    /// Members have the same structural (Merkle) shape.
    SameShape,
    /// Root is inside a `nav` / `navigation` landmark.
    InNavigation,
    /// Popup content is mostly links.
    PopupMostlyLinks,
    /// The root is an editable element (input/textarea).
    Editable,
    /// `aria-controls` / `aria-owns` resolves to an element with this role.
    ControlsRole(&'static str),
    /// Native tag implements the semantics (`input[type=checkbox]`, `select`).
    Native,
    /// The trigger's popup (`aria-controls`, else the next sibling) has this role.
    PopupRole(&'static str),
    /// A native `<input type=...>` (the root, or every member when there are members).
    NativeInput(&'static str),
    /// The member triggers carry `aria-haspopup`.
    HasPopupMembers,
    /// Every member contains a button or link to act as the trigger.
    TriggerInside,
}

#[derive(Debug, Clone, Copy)]
pub struct Check {
    pub name: &'static str,
    pub kind: CheckKind,
    pub weight: f32,
    pub pred: Pred,
}

#[derive(Debug, Clone, Copy)]
pub struct Rule {
    pub id: &'static str,
    pub primitive: Primitive,
    pub summary: &'static str,
    pub root: RootSel,
    pub checks: &'static [Check],
    /// Lowest accepted confidence.
    pub threshold: f32,
}

const fn req(name: &'static str, pred: Pred) -> Check {
    Check {
        name,
        kind: CheckKind::Required,
        weight: 0.0,
        pred,
    }
}
const fn sup(name: &'static str, weight: f32, pred: Pred) -> Check {
    Check {
        name,
        kind: CheckKind::Supporting,
        weight,
        pred,
    }
}
const fn con(name: &'static str, weight: f32, pred: Pred) -> Check {
    Check {
        name,
        kind: CheckKind::Contra,
        weight,
        pred,
    }
}

/// Order matters: an element is claimed by the first rule that accepts it.
pub static RULES: &[Rule] = &[
    Rule {
        id: "alert-dialog.role",
        primitive: Primitive::AlertDialog,
        summary: "role=alertdialog is a modal that interrupts and requires a response",
        root: RootSel::Role(&["alertdialog"]),
        checks: &[
            req("role alertdialog", Pred::RoleIs("alertdialog")),
            sup("aria-modal", 0.3, Pred::AriaTrue("aria-modal")),
            sup("layered above the page", 0.2, Pred::Layered),
            sup("closed in the initial capture", 0.1, Pred::InitiallyClosed),
            sup("state recovered from the bundle", 0.25, Pred::JsBound),
            sup("accessible name", 0.15, Pred::Named),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "dialog.role",
        primitive: Primitive::Dialog,
        summary: "role=dialog (or <dialog>) with modal, inert and layering evidence",
        root: RootSel::Role(&["dialog"]),
        checks: &[
            req("role dialog", Pred::RoleIs("dialog")),
            sup("aria-modal", 0.3, Pred::AriaTrue("aria-modal")),
            sup("layered above the page", 0.2, Pred::Layered),
            sup("closed in the initial capture", 0.1, Pred::InitiallyClosed),
            sup("state recovered from the bundle", 0.25, Pred::JsBound),
            sup("accessible name", 0.15, Pred::Named),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "tabs.tablist",
        primitive: Primitive::Tabs,
        summary: "tablist owning tab elements, usually with tabpanels wired by aria-controls",
        root: RootSel::Role(&["tablist"]),
        checks: &[
            req("role tablist", Pred::RoleIs("tablist")),
            req(
                "owns tabs",
                Pred::Members {
                    min: 1,
                    max: usize::MAX,
                },
            ),
            sup("exactly one tab selected", 0.2, Pred::OneSelected),
            sup("tabs control panels", 0.3, Pred::MembersControlPanels),
            sup("accessible name", 0.1, Pred::Named),
            sup("state recovered from the bundle", 0.2, Pred::JsBound),
            sup("same-shaped tabs", 0.2, Pred::SameShape),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "combobox.editable",
        primitive: Primitive::Combobox,
        summary: "role=combobox on an editable element with a popup list",
        root: RootSel::Role(&["combobox"]),
        checks: &[
            req("role combobox", Pred::RoleIs("combobox")),
            req("editable", Pred::Editable),
            sup("controls a listbox", 0.4, Pred::ControlsRole("listbox")),
            sup("aria-expanded", 0.2, Pred::AttrPresent("aria-expanded")),
            sup("accessible name", 0.2, Pred::Named),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "select.combobox",
        primitive: Primitive::Select,
        summary: "non-editable combobox, native <select> or listbox trigger",
        root: RootSel::Role(&["combobox", "listbox"]),
        checks: &[
            con("editable", 1.0, Pred::Editable),
            sup("native select", 0.3, Pred::TagIs("select")),
            sup("controls a listbox", 0.3, Pred::ControlsRole("listbox")),
            sup("aria-expanded", 0.2, Pred::AttrPresent("aria-expanded")),
            sup("accessible name", 0.2, Pred::Named),
        ],
        threshold: 0.35,
    },
    Rule {
        id: "menubar.role",
        primitive: Primitive::Menubar,
        summary: "role=menubar owning menuitems",
        root: RootSel::Role(&["menubar"]),
        checks: &[
            req("role menubar", Pred::RoleIs("menubar")),
            req(
                "owns menu items",
                Pred::Members {
                    min: 1,
                    max: usize::MAX,
                },
            ),
            sup("accessible name", 0.2, Pred::Named),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "menu.role",
        primitive: Primitive::Menu,
        summary: "role=menu popup owning menuitems, opened by a trigger",
        root: RootSel::Role(&["menu"]),
        checks: &[
            req("role menu", Pred::RoleIs("menu")),
            req(
                "owns menu items",
                Pred::Members {
                    min: 1,
                    max: usize::MAX,
                },
            ),
            sup("accessible name", 0.2, Pred::Named),
            sup("closed in the initial capture", 0.1, Pred::InitiallyClosed),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "navigation-menu.popup-links",
        primitive: Primitive::NavigationMenu,
        summary: "navigation landmark whose triggers open panels of links",
        root: RootSel::PopupTrigger,
        checks: &[
            req("trigger has aria-haspopup", Pred::HasPopupMembers),
            req("inside a navigation landmark", Pred::InNavigation),
            sup("popup is mostly links", 0.4, Pred::PopupMostlyLinks),
            sup(
                "aria-expanded present",
                0.2,
                Pred::AttrPresent("aria-expanded"),
            ),
            sup("accessible name", 0.2, Pred::Named),
        ],
        threshold: 0.55,
    },
    Rule {
        id: "navigation-menu.hover-state",
        primitive: Primitive::NavigationMenu,
        summary: "items whose pointer-enter handler opens one shared dropdown by key (state in the bundle)",
        root: RootSel::HoverMenu,
        checks: &[
            req(
                "two or more hover items",
                Pred::Members {
                    min: 2,
                    max: usize::MAX,
                },
            ),
            req("state recovered from the bundle", Pred::JsBound),
            sup("each item holds a button or link", 0.3, Pred::TriggerInside),
            sup("inside a navigation landmark", 0.3, Pred::InNavigation),
            sup("same-shaped items", 0.2, Pred::SameShape),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "popover.haspopup",
        primitive: Primitive::Popover,
        summary: "aria-haspopup trigger whose popup is not a menu or a list of links",
        root: RootSel::PopupTrigger,
        checks: &[
            req("trigger has aria-haspopup", Pred::HasPopupMembers),
            sup(
                "aria-expanded present",
                0.3,
                Pred::AttrPresent("aria-expanded"),
            ),
            con("popup is mostly links", 0.3, Pred::PopupMostlyLinks),
            con(
                "controls a menu (a Menu pattern owns it)",
                1.0,
                Pred::PopupRole("menu"),
            ),
            sup("accessible name", 0.2, Pred::Named),
        ],
        threshold: 0.45,
    },
    Rule {
        id: "switch.role",
        primitive: Primitive::Switch,
        summary: "role=switch (native checkbox or custom element) with aria-checked",
        root: RootSel::Role(&["switch"]),
        checks: &[
            req("role switch", Pred::RoleIs("switch")),
            sup("aria-checked", 0.3, Pred::AttrPresent("aria-checked")),
            sup("accessible name", 0.3, Pred::Named),
            sup("native checkbox input", 0.2, Pred::NativeInput("checkbox")),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "checkbox.role",
        primitive: Primitive::Checkbox,
        summary: "checkbox role (native input or custom element)",
        root: RootSel::Role(&["checkbox"]),
        checks: &[
            req("role checkbox", Pred::RoleIs("checkbox")),
            sup("accessible name", 0.4, Pred::Named),
            sup("native checkbox input", 0.3, Pred::NativeInput("checkbox")),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "radio-group.role",
        primitive: Primitive::RadioGroup,
        summary: "radiogroup of radios, or native radios sharing a name",
        root: RootSel::Role(&["radiogroup"]),
        checks: &[
            req("role radiogroup", Pred::RoleIs("radiogroup")),
            req(
                "owns radios",
                Pred::Members {
                    min: 1,
                    max: usize::MAX,
                },
            ),
            sup("one selected", 0.3, Pred::OneSelected),
            sup("accessible name", 0.3, Pred::Named),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "radio-group.native-name",
        primitive: Primitive::RadioGroup,
        summary: "native radio inputs sharing a name attribute",
        root: RootSel::RadioNames,
        checks: &[
            req(
                "at least two radios",
                Pred::Members {
                    min: 2,
                    max: usize::MAX,
                },
            ),
            sup("native radio", 0.5, Pred::NativeInput("radio")),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "tooltip.role",
        primitive: Primitive::Tooltip,
        summary: "role=tooltip referenced by aria-describedby",
        root: RootSel::Role(&["tooltip"]),
        checks: &[
            req("role tooltip", Pred::RoleIs("tooltip")),
            sup("closed in the initial capture", 0.2, Pred::InitiallyClosed),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "toggle-group.pressed",
        primitive: Primitive::ToggleGroup,
        summary: "sibling buttons with aria-pressed (single-select when one is pressed)",
        root: RootSel::PressedGroup,
        checks: &[
            req(
                "at least two pressed-state buttons",
                Pred::Members {
                    min: 2,
                    max: usize::MAX,
                },
            ),
            sup("exactly one pressed", 0.3, Pred::OneSelected),
            sup("same-shaped buttons", 0.3, Pred::SameShape),
            sup("state recovered from the bundle", 0.2, Pred::JsBound),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "toggle.pressed",
        primitive: Primitive::Toggle,
        summary: "single button with aria-pressed",
        root: RootSel::PressedGroup,
        checks: &[
            req(
                "single pressed-state button",
                Pred::Members { min: 1, max: 1 },
            ),
            sup("state recovered from the bundle", 0.3, Pred::JsBound),
            sup("accessible name", 0.3, Pred::Named),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "accordion.disclosure-group",
        primitive: Primitive::Accordion,
        summary: "two or more aria-expanded triggers with sibling panels under one parent",
        root: RootSel::Disclosure,
        checks: &[
            req(
                "two or more disclosures",
                Pred::Members {
                    min: 2,
                    max: usize::MAX,
                },
            ),
            req("every trigger has a panel", Pred::PanelsResolved),
            sup(
                "panels wired by aria-controls",
                0.3,
                Pred::PanelsLinkedByAria,
            ),
            sup(
                "collapsed panels are inert/hidden",
                0.2,
                Pred::PanelsClosedAreInert,
            ),
            sup("same-shaped items", 0.3, Pred::SameShape),
            sup("state recovered from the bundle", 0.2, Pred::JsBound),
            con("triggers open popups", 1.0, Pred::HasPopupMembers),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "collapsible.disclosure",
        primitive: Primitive::Collapsible,
        summary: "one aria-expanded trigger with a panel",
        root: RootSel::Disclosure,
        checks: &[
            req("one disclosure", Pred::Members { min: 1, max: 1 }),
            req("trigger has a panel", Pred::PanelsResolved),
            sup(
                "panel wired by aria-controls",
                0.3,
                Pred::PanelsLinkedByAria,
            ),
            sup(
                "collapsed panel is inert/hidden",
                0.2,
                Pred::PanelsClosedAreInert,
            ),
            sup("state recovered from the bundle", 0.2, Pred::JsBound),
            sup("accessible name", 0.2, Pred::Named),
            con("trigger opens a popup", 1.0, Pred::HasPopupMembers),
        ],
        threshold: 0.5,
    },
    Rule {
        id: "field.native",
        primitive: Primitive::Field,
        summary: "native text-like input, textarea or select with a label",
        root: RootSel::FormField,
        checks: &[
            req("native form control", Pred::Native),
            sup("accessible name", 0.5, Pred::Named),
        ],
        threshold: 0.3,
    },
    Rule {
        id: "button.native",
        primitive: Primitive::Button,
        summary: "button element or role=button",
        root: RootSel::ButtonLike,
        checks: &[
            req("button role", Pred::RoleIs("button")),
            sup("native button", 0.5, Pred::TagIs("button")),
            sup("accessible name", 0.3, Pred::Named),
        ],
        threshold: 0.3,
    },
];

pub struct Ctx<'a> {
    pub doc: &'a Doc,
    pub js: &'a [JsFacts],
    pub links: &'a Links,
}

fn text_is_link_heavy(doc: &Doc, popup: NodeId) -> bool {
    let controls = doc.interactive_descendants(popup);
    if controls.is_empty() {
        return false;
    }
    let links = controls
        .iter()
        .filter(|c| doc.node(**c).role() == Some("link"))
        .count();
    links * 10 >= controls.len() * 7
}

/// The popup element for a trigger: `aria-controls`, else the next element sibling.
pub fn popup_of(doc: &Doc, trigger: NodeId) -> Option<NodeId> {
    if let Some(c) = doc.controlled(trigger).into_iter().next() {
        return Some(c);
    }
    let parent = doc.node(trigger).parent?;
    let siblings: Vec<NodeId> = doc.element_children(parent).collect();
    let pos = siblings.iter().position(|s| *s == trigger)?;
    siblings.get(pos + 1).copied()
}

/// Evaluate one predicate; `Some(detail)` when it holds.
pub fn eval(pred: Pred, cx: &Ctx<'_>, cand: &Candidate) -> Option<(EvidenceSource, String)> {
    use EvidenceSource::*;
    let doc = cx.doc;
    let root = doc.node(cand.root);
    let first = cand.members.first().copied().unwrap_or(cand.root);
    Some(match pred {
        Pred::RoleIs(r) => (root.role() == Some(r)).then(|| {
            (
                Aria,
                format!("effective role `{r}` ({:?})", root.role_source),
            )
        })?,
        Pred::TagIs(t) => (root.tag == t).then(|| (Dom, format!("element <{t}>")))?,
        Pred::AriaTrue(a) => {
            (root.aria_bool(a) == Some(true)).then(|| (Aria, format!("{a}=\"true\"")))?
        }
        Pred::AttrPresent(a) => {
            let node = if cand.members.is_empty() {
                root
            } else {
                doc.node(first)
            };
            node.has_attr(a).then(|| (Aria, format!("{a} present")))?
        }
        Pred::Members { min, max } => {
            let n = cand.members.len();
            (n >= min && n <= max).then(|| (Structure, format!("{n} member(s)")))?
        }
        Pred::OneSelected => {
            let selected = cand
                .members
                .iter()
                .filter(|m| {
                    let n = doc.node(**m);
                    n.aria_bool("aria-selected") == Some(true)
                        || n.aria_bool("aria-pressed") == Some(true)
                        || n.aria_bool("aria-checked") == Some(true)
                        || n.has_attr("checked")
                })
                .count();
            (selected == 1).then(|| {
                (
                    Aria,
                    "exactly one member selected in the capture".to_owned(),
                )
            })?
        }
        Pred::MembersControlPanels => {
            let ok = !cand.members.is_empty()
                && cand.members.iter().all(|m| !doc.controlled(*m).is_empty());
            ok.then(|| (Aria, "every member's aria-controls resolves".to_owned()))?
        }
        Pred::PanelsResolved => {
            let ok = !cand.members.is_empty()
                && cand.panels.len() == cand.members.len()
                && cand.panels.iter().all(Option::is_some);
            ok.then(|| {
                (
                    Structure,
                    format!("panels resolved via {}", cand.panel_via.join(", ")),
                )
            })?
        }
        Pred::PanelsLinkedByAria => {
            let ok = !cand.members.is_empty()
                && cand
                    .panel_via
                    .iter()
                    .all(|v| matches!(*v, "aria-controls" | "labelledby"));
            ok.then(|| {
                (
                    Aria,
                    "panels referenced through aria-controls/aria-labelledby".to_owned(),
                )
            })?
        }
        Pred::PanelsClosedAreInert => {
            let collapsed: Vec<NodeId> = cand
                .members
                .iter()
                .zip(&cand.panels)
                .filter(|(m, _)| doc.node(**m).aria_bool("aria-expanded") == Some(false))
                .filter_map(|(_, p)| *p)
                .collect();
            let ok =
                !collapsed.is_empty() && collapsed.iter().all(|p| doc.inert(*p) || doc.hidden(*p));
            ok.then(|| {
                (
                    Style,
                    "collapsed panels are inert/hidden in the capture".to_owned(),
                )
            })?
        }
        Pred::Layered => {
            let pos = root.style("position");
            let z = root
                .style("z-index")
                .and_then(|z| z.parse::<i64>().ok())
                .unwrap_or(0);
            (pos == Some("fixed") || (pos == Some("absolute") && z > 0))
                .then(|| (Style, format!("position {} z-index {z}", pos.unwrap_or(""))))?
        }
        Pred::InitiallyClosed => {
            (doc.inert(cand.root) || doc.hidden(cand.root) || root.has_attr("hidden"))
                .then(|| (Dom, "inert/hidden in the captured initial state".to_owned()))?
        }
        Pred::JsBound => {
            let hit = std::iter::once(cand.root)
                .chain(cand.members.iter().copied())
                .flat_map(|n| cx.links.of(n))
                .find(|l| {
                    let site = &cx.js[l.file].sites[l.site];
                    !site.bindings.is_empty()
                        || !site.handlers.is_empty()
                        || !site.conditional_props.is_empty()
                        || !site.class_conditional.is_empty()
                })?;
            let (file, site) = (&cx.js[hit.file], &cx.js[hit.file].sites[hit.site]);
            (
                Js,
                format!(
                    "site `{}` (line {}) in {} has bound state or handlers",
                    site.tag, site.line, file.file
                ),
            )
        }
        Pred::Named => {
            let n = if root.name.is_some() {
                root
            } else {
                doc.node(first)
            };
            n.name.as_ref().map(|name| {
                (
                    Aria,
                    format!("accessible name `{name}` ({:?})", n.name_source),
                )
            })?
        }
        Pred::SameShape => {
            let shapes: Vec<u64> = cand
                .members
                .iter()
                .map(|m| crate::semantic::components::shape_hash(doc, *m))
                .collect();
            (shapes.len() >= 2 && shapes.windows(2).all(|w| w[0] == w[1]))
                .then(|| (Structure, "members share one structural hash".to_owned()))?
        }
        Pred::InNavigation => doc
            .ancestors(cand.root)
            .chain(std::iter::once(cand.root))
            .any(|a| doc.node(a).role() == Some("navigation"))
            .then(|| (Aria, "inside a navigation landmark".to_owned()))?,
        Pred::PopupMostlyLinks => {
            let ok = cand
                .members
                .iter()
                .filter_map(|m| popup_of(doc, *m))
                .any(|p| text_is_link_heavy(doc, p));
            ok.then(|| (Structure, "popup content is at least 70% links".to_owned()))?
        }
        Pred::Editable => (matches!(root.tag.as_str(), "input" | "textarea")
            || root.attr("contenteditable") == Some("true"))
        .then(|| (Dom, format!("editable <{}>", root.tag)))?,
        Pred::ControlsRole(role) => {
            let target = doc
                .controlled(cand.root)
                .into_iter()
                .chain(
                    root.id_refs("aria-owns")
                        .into_iter()
                        .filter_map(|r| doc.resolve(r)),
                )
                .find(|t| doc.node(*t).role() == Some(role))?;
            (Aria, format!("controls node {target} with role {role}"))
        }
        Pred::Native => {
            let native = matches!(
                root.tag.as_str(),
                "input" | "select" | "textarea" | "button"
            );
            native.then(|| (Dom, format!("native <{}>", root.tag)))?
        }
        Pred::PopupRole(role) => {
            let popup = cand.members.iter().find_map(|m| popup_of(doc, *m))?;
            (doc.node(popup).role() == Some(role))
                .then(|| (Aria, format!("popup node {popup} has role {role}")))?
        }
        Pred::NativeInput(kind) => {
            let nodes: Vec<NodeId> = if cand.members.is_empty() {
                vec![cand.root]
            } else {
                cand.members.clone()
            };
            nodes
                .iter()
                .all(|n| {
                    let node = doc.node(*n);
                    node.tag == "input" && node.attr("type") == Some(kind)
                })
                .then(|| (Dom, format!("native <input type=\"{kind}\">")))?
        }
        Pred::TriggerInside => {
            let ok = !cand.members.is_empty()
                && cand.members.iter().all(|m| {
                    doc.descendants(*m)
                        .into_iter()
                        .chain(std::iter::once(*m))
                        .any(|d| matches!(doc.node(d).role(), Some("button" | "link")))
                });
            ok.then(|| (Structure, "every item contains a button or link".to_owned()))?
        }
        Pred::HasPopupMembers => {
            let ok = !cand.members.is_empty()
                && cand.members.iter().all(|m| {
                    doc.node(*m)
                        .attr("aria-haspopup")
                        .is_some_and(|v| v != "false")
                });
            ok.then(|| (Aria, "triggers carry aria-haspopup".to_owned()))?
        }
    })
}
