//! Component names for emitted compositions: `data-component` of the nearest
//! element, else the root's BEM class block, else the accessible name; never an
//! index or a hash. Returns the name with the evidence it came from.
use super::ts::join_words;
use crate::semantic::ir::Doc;
use crate::semantic::patterns::{Match, Primitive};

fn noun(p: Primitive) -> &'static str {
    match p {
        Primitive::Tabs => "Tabs",
        Primitive::ToggleGroup => "ToggleGroup",
        Primitive::Toggle => "Toggle",
        Primitive::Accordion => "Accordion",
        Primitive::Collapsible => "Collapsible",
        Primitive::Dialog => "Dialog",
        Primitive::AlertDialog => "AlertDialog",
        Primitive::NavigationMenu => "NavigationMenu",
        Primitive::Switch => "Switch",
        Primitive::Checkbox => "Checkbox",
        Primitive::RadioGroup => "RadioGroup",
        Primitive::Popover => "Popover",
        Primitive::Tooltip => "Tooltip",
        Primitive::Menu => "Menu",
        Primitive::Menubar => "Menubar",
        Primitive::Select => "Select",
        Primitive::Combobox => "Combobox",
        Primitive::Field => "Field",
        Primitive::Button => "Button",
    }
}

/// Words that already say what the component is, so the noun is not appended.
const SELF_DESCRIBING: &[&str] = &[
    "sheet",
    "modal",
    "dialog",
    "drawer",
    "tabs",
    "menu",
    "accordion",
    "switch",
    "toggle",
    "collapsible",
    "popover",
    "tooltip",
];

pub fn component_name(doc: &Doc, m: &Match) -> Option<(String, String)> {
    let root = doc.node(m.root);
    let own = std::iter::once(m.root)
        .chain(doc.ancestors(m.root))
        .find_map(|n| {
            doc.node(n)
                .attr("data-component")
                .map(|d| (d.to_owned(), n))
        });
    let class = root
        .classes
        .iter()
        .filter(|c| {
            !crate::semantic::components::is_state_class(c)
                && !crate::semantic::components::is_utility_class(c)
        })
        .max_by_key(|c| c.len())
        .cloned();
    let n = noun(m.primitive);
    let (base, source): (String, String) = if let Some((dc, _)) = own {
        let label = m.label.clone().unwrap_or_default();
        (
            join_words(&[&dc, &label, n]),
            format!(
                "data-component `{dc}`{}",
                if label.is_empty() {
                    String::new()
                } else {
                    format!(" + accessible name `{label}`")
                }
            ),
        )
    } else if let Some(c) = class.filter(|c| c.len() >= 3) {
        let words = crate::semantic::components::words(&c);
        let describing = words
            .last()
            .is_some_and(|w| SELF_DESCRIBING.contains(&w.as_str()));
        let name = if describing {
            join_words(&[&c])
        } else {
            join_words(&[&c, n])
        };
        (name, format!("root class `{c}`"))
    } else {
        let label = m.label.clone().filter(|l| !l.is_empty())?;
        (
            join_words(&[&label, n]),
            format!("accessible name `{label}`"),
        )
    };
    let valid = base.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && base.chars().all(|c| c.is_ascii_alphanumeric());
    valid.then_some((base, source))
}
