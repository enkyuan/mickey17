use super::*;
use anyhow::{Result, bail, ensure};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.starts_with(|c: char| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(
            s,
            "__proto__"
                | "constructor"
                | "prototype"
                | "default"
                | "class"
                | "return"
                | "function"
                | "const"
                | "let"
                | "var"
                | "new"
                | "this"
                | "import"
                | "export"
                | "await"
                | "yield"
                | "delete"
                | "typeof"
                | "void"
                | "in"
                | "instanceof"
                | "extends"
                | "super"
                | "null"
                | "true"
                | "false"
                | "break"
                | "case"
                | "catch"
                | "continue"
                | "debugger"
                | "do"
                | "else"
                | "finally"
                | "for"
                | "if"
                | "switch"
                | "throw"
                | "try"
                | "while"
                | "with"
                | "enum"
                | "implements"
                | "interface"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "static"
        )
}
fn reserved_runtime_binding(name: &str) -> bool {
    matches!(
        name,
        "scope"
            | "motion"
            | "useRef"
            | "ComponentRef"
            | "useMotionPreference"
            | "useScrollHeader"
            | "useScrollReveal"
            | "Fragment"
            | "useState"
            | "Link"
            | "assets"
            | "styles"
            | "props"
            | "NavigationMenu"
            | "Menu"
            | "Dialog"
            | "Field"
            | "Switch"
            | "Tabs"
            | "Form"
            | "Input"
            | "Button"
    )
}
fn reserved_binding(name: &str, s: &Specification) -> bool {
    reserved_runtime_binding(name)
        || s.components.contains_key(name)
        || s.external_components.contains_key(name)
}
fn token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}
fn href(s: &str) -> bool {
    !s.chars().any(char::is_control)
        && (s.starts_with('/') && !s.starts_with("//")
            || s.starts_with('#')
            || url::Url::parse(s).is_ok_and(|u| {
                matches!(u.scheme(), "https" | "http" | "mailto" | "tel")
                    && u.username().is_empty()
                    && u.password().is_none()
            }))
}
pub(super) fn primitive(name: &str) -> Option<&'static str> {
    Some(match name {
        "NavigationMenu.Root"
        | "NavigationMenu.List"
        | "NavigationMenu.Item"
        | "NavigationMenu.Trigger"
        | "NavigationMenu.Content"
        | "NavigationMenu.Portal"
        | "NavigationMenu.Positioner"
        | "NavigationMenu.Popup"
        | "NavigationMenu.Viewport"
        | "NavigationMenu.Link"
        | "NavigationMenu.Icon" => "navigation-menu",
        "Menu.Root" | "Menu.Trigger" | "Menu.Portal" | "Menu.Positioner" | "Menu.Popup"
        | "Menu.Item" | "Menu.Separator" => "menu",
        "Dialog.Root" | "Dialog.Trigger" | "Dialog.Portal" | "Dialog.Backdrop" | "Dialog.Popup"
        | "Dialog.Title" | "Dialog.Description" | "Dialog.Close" => "dialog",
        "Field.Root" | "Field.Label" | "Field.Control" | "Field.Description" | "Field.Error" => {
            "field"
        }
        "Switch.Root" | "Switch.Thumb" => "switch",
        "Tabs.Root" | "Tabs.List" | "Tabs.Tab" | "Tabs.Panel" => "tabs",
        "Form" => "form",
        "Input" => "input",
        "Button" => "button",
        _ => return None,
    })
}
fn shape(t: &Type, depth: usize) -> Result<()> {
    ensure!(depth <= 8, "type nesting budget exceeded");
    match t {
        Type::List { item } => shape(item, depth + 1)?,
        Type::Object { fields } => {
            ensure!(fields.len() <= 32, "too many typed fields");
            for (k, v) in fields {
                ensure!(identifier(k), "invalid field name {k}");
                shape(v, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub(super) fn data(t: &Type, v: &Value, assets: &BTreeMap<String, String>) -> Result<()> {
    let valid = match (t, v) {
        (Type::Text, Value::String(s)) => s.len() <= 65536,
        (Type::Href, Value::String(s)) => href(s),
        (Type::Asset, Value::String(s)) => assets.contains_key(s),
        (Type::Number, Value::Number(_)) | (Type::Boolean, Value::Bool(_)) => true,
        (Type::List { item }, Value::Array(values)) => {
            ensure!(values.len() <= 512, "data list budget exceeded");
            for value in values {
                data(item, value, assets)?;
            }
            true
        }
        (Type::Object { fields }, Value::Object(values)) => {
            ensure!(
                fields.keys().eq(values.keys()),
                "typed object fields differ"
            );
            for (k, t) in fields {
                data(t, &values[k], assets)?;
            }
            true
        }
        _ => false,
    };
    ensure!(valid, "value does not satisfy {t:?}");
    Ok(())
}
pub(super) fn reference(path: &str, env: &BTreeMap<String, Type>) -> Result<Type> {
    let mut parts = path.split('.');
    let first = parts.next().unwrap_or("");
    let mut t = env
        .get(first)
        .with_context(|| format!("unknown binding {first}"))?
        .clone();
    for part in parts {
        ensure!(identifier(part), "invalid property path");
        let Type::Object { fields } = t else {
            bail!("property access on non-object")
        };
        t = fields.get(part).context("unknown property")?.clone();
    }
    Ok(t)
}
fn expression(
    expr: &Expression,
    expected: &Type,
    env: &BTreeMap<String, Type>,
    s: &Specification,
) -> Result<()> {
    match expr {
        Expression::Object { fields } => {
            let Type::Object { fields: types } = expected else {
                bail!("object expression needs object type")
            };
            ensure!(
                fields.keys().eq(types.keys()),
                "object expression fields differ"
            );
            for (k, v) in fields {
                expression(v, &types[k], env, s)?;
            }
            Ok(())
        }
        Expression::List { items } => {
            let Type::List { item } = expected else {
                bail!("list expression needs list type")
            };
            ensure!(items.len() <= 512, "expression list budget exceeded");
            for v in items {
                expression(v, item, env, s)?;
            }
            Ok(())
        }

        Expression::Preference { value } => {
            ensure!(
                !matches!(value, Preference::SetAccessibilityMode) && *expected == Type::Boolean,
                "preference requires boolean data"
            );
            Ok(())
        }
        Expression::Literal { value } => data(expected, value, &s.assets),
        Expression::Ref { path } => {
            let actual = reference(path, env)?;
            ensure!(
                actual == *expected
                    || *expected == Type::Text && matches!(actual, Type::Href | Type::Asset),
                "binding type mismatch at {path}"
            );
            Ok(())
        }
        Expression::Asset { name } => {
            ensure!(
                s.assets.contains_key(name) && matches!(expected, Type::Asset | Type::Text),
                "unknown asset binding"
            );
            Ok(())
        }
        _ => bail!("handler/render expression in a data position"),
    }
}
fn prop(
    expr: &Expression,
    key: &str,
    env: &BTreeMap<String, Type>,
    c: &Component,
    s: &Specification,
) -> Result<()> {
    ensure!(
        identifier(key) || (key.starts_with("aria-") || key.starts_with("data-")) && token(key),
        "invalid JSX property"
    );
    ensure!(
        !matches!(
            key,
            "dangerouslySetInnerHTML"
                | "innerHTML"
                | "srcDoc"
                | "style"
                | "className"
                | "ref"
                | "children"
        ),
        "opaque JSX property refused: {key}"
    );
    if key.starts_with("on") {
        match expr {
            Expression::Assign { state, value } => {
                ensure!(
                    matches!(key, "onClick" | "onMouseEnter" | "onFocus"),
                    "unsupported bound state event"
                );
                expression(
                    value,
                    &c.state
                        .get(state)
                        .context("unknown state assignment")?
                        .value_type,
                    env,
                    s,
                )?;
            }
            Expression::Preference {
                value: Preference::SetAccessibilityMode,
            } => ensure!(
                key == "onCheckedChange",
                "preference setter requires checked change"
            ),
            Expression::Setter { state } => {
                ensure!(
                    matches!(key, "onOpenChange" | "onValueChange" | "onCheckedChange"),
                    "unsupported state setter event"
                );
                ensure!(c.state.contains_key(state), "unknown state setter");
            }
            Expression::Set {
                state,
                value,
                prevent_default,
            } => {
                ensure!(
                    matches!(key, "onClick" | "onSubmit"),
                    "unsupported local event"
                );
                ensure!(
                    key != "onSubmit" || *prevent_default,
                    "local form handler must prevent network submission"
                );
                data(
                    &c.state
                        .get(state)
                        .context("unknown state action")?
                        .value_type,
                    value,
                    &s.assets,
                )?;
            }
            _ => bail!("event must be an explicit state transition"),
        }
        return Ok(());
    }
    if let Expression::Link { href } = expr {
        ensure!(key == "render", "render element outside render prop");
        return expression(href, &Type::Href, env, s);
    }
    ensure!(
        !matches!(
            expr,
            Expression::Setter { .. } | Expression::Set { .. } | Expression::Assign { .. }
        ),
        "handler outside event prop"
    );
    if matches!(key, "href" | "to" | "action" | "formAction") {
        return expression(expr, &Type::Href, env, s);
    }
    if matches!(key, "src" | "poster") {
        return expression(expr, &Type::Asset, env, s);
    }
    match expr {
        Expression::Literal { value } => ensure!(
            value.is_string() || value.is_number() || value.is_boolean(),
            "JSX property must be scalar"
        ),
        Expression::Ref { path } => ensure!(
            matches!(
                reference(path, env)?,
                Type::Text | Type::Number | Type::Boolean | Type::Href | Type::Asset
            ),
            "JSX binding must be scalar"
        ),
        Expression::Asset { name } => ensure!(s.assets.contains_key(name), "unknown asset"),
        Expression::Preference { value } => ensure!(
            !matches!(value, Preference::SetAccessibilityMode),
            "preference setter outside event"
        ),
        _ => bail!("unsupported expression"),
    };
    Ok(())
}
fn parts(n: &Node, out: &mut BTreeSet<String>) {
    match n {
        Node::If {
            then, otherwise, ..
        } => {
            parts(then, out);
            parts(otherwise, out);
        }
        Node::Primitive { name, children, .. } => {
            out.insert(name.clone());
            for n in children {
                parts(n, out)
            }
        }
        Node::Element { children, .. } => {
            for n in children {
                parts(n, out)
            }
        }
        Node::Repeat { body, .. } => parts(body, out),
        _ => {}
    }
}
fn node(
    n: &Node,
    env: &BTreeMap<String, Type>,
    c: &Component,
    s: &Specification,
    count: &mut usize,
    depth: usize,
    edges: &mut BTreeSet<String>,
) -> Result<()> {
    *count += 1;
    ensure!(
        *count <= 8192 && depth <= 64,
        "authored node budget exceeded"
    );
    match n {
        Node::If {
            condition,
            then,
            otherwise,
        } => {
            expression(condition, &Type::Boolean, env, s)?;
            node(then, env, c, s, count, depth + 1, edges)?;
            node(otherwise, env, c, s, count, depth + 1, edges)?;
        }
        Node::Text { value } => {
            let t = if let Expression::Ref { path } = value {
                reference(path, env)?
            } else {
                Type::Text
            };
            ensure!(
                matches!(t, Type::Text | Type::Number | Type::Href),
                "non-renderable text value"
            );
            expression(value, &t, env, s)?;
        }
        Node::External { name, props } => {
            let target = s
                .external_components
                .get(name)
                .context("unknown owned component")?;
            ensure!(
                props.keys().eq(target.props.keys()),
                "owned component props differ"
            );
            for (key, value) in props {
                expression(value, &target.props[key], env, s)?;
            }
        }
        Node::Component { name, props } => {
            let target = s
                .components
                .get(name)
                .context("unknown component reference")?;
            ensure!(
                props.keys().eq(target.props.keys()),
                "component props differ for {name}"
            );
            for (k, v) in props {
                expression(v, &target.props[k], env, s)?;
            }
            edges.insert(name.clone());
        }
        Node::Repeat {
            each,
            binding,
            key,
            body,
        } => {
            ensure!(
                identifier(binding) && !env.contains_key(binding) && !reserved_binding(binding, s),
                "invalid or shadowed repeat binding"
            );
            let Expression::Ref { path } = each else {
                bail!("repeat needs a typed data binding")
            };
            let Type::List { item } = reference(path, env)? else {
                bail!("repeat needs a list")
            };
            let mut inner = env.clone();
            inner.insert(binding.clone(), *item);
            let Expression::Ref { path } = key else {
                bail!("repeat key must reference an item field")
            };
            ensure!(
                path.starts_with(&format!("{binding}.")) || path == binding,
                "repeat key must belong to the item"
            );
            ensure!(
                matches!(
                    reference(path, &inner)?,
                    Type::Text | Type::Href | Type::Number
                ),
                "invalid repeat key type"
            );
            node(body, &inner, c, s, count, depth + 1, edges)?;
        }
        Node::Element {
            tag: _,
            props,
            classes,
            children,
        }
        | Node::Primitive {
            name: _,
            props,
            classes,
            children,
        } => {
            if let Node::Element { tag, .. } = n {
                ensure!(
                    matches!(
                        tag.as_str(),
                        "div"
                            | "span"
                            | "main"
                            | "header"
                            | "footer"
                            | "section"
                            | "article"
                            | "nav"
                            | "aside"
                            | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "p"
                            | "a"
                            | "ul"
                            | "ol"
                            | "li"
                            | "img"
                            | "strong"
                            | "em"
                            | "small"
                            | "br"
                            | "hr"
                            | "svg"
                            | "path"
                            | "circle"
                            | "line"
                            | "g"
                            | "title"
                    ),
                    "unsupported intrinsic element {tag}; use a Base UI control"
                );
                ensure!(
                    !props.keys().any(|k| k.starts_with("on")),
                    "native behavior requires an explicit primitive"
                );
            }
            if let Node::Primitive { name, .. } = n {
                ensure!(
                    primitive(name).is_some(),
                    "unsupported Base UI primitive {name}"
                );
                let mut present = BTreeSet::new();
                parts(n, &mut present);
                let required: &[&str] = match name.as_str() {
                    "Menu.Root" => &[
                        "Menu.Trigger",
                        "Menu.Portal",
                        "Menu.Positioner",
                        "Menu.Popup",
                    ],
                    "Dialog.Root" => &[
                        "Dialog.Trigger",
                        "Dialog.Portal",
                        "Dialog.Popup",
                        "Dialog.Title",
                        "Dialog.Close",
                    ],
                    "NavigationMenu.Root" if present.contains("NavigationMenu.Trigger") => &[
                        "NavigationMenu.List",
                        "NavigationMenu.Item",
                        "NavigationMenu.Portal",
                        "NavigationMenu.Positioner",
                        "NavigationMenu.Popup",
                        "NavigationMenu.Viewport",
                    ],
                    "NavigationMenu.Root" => &["NavigationMenu.List", "NavigationMenu.Item"],
                    "Tabs.Root" => &["Tabs.List", "Tabs.Tab", "Tabs.Panel"],
                    "Field.Root" => &["Field.Label", "Field.Control"],
                    _ => &[],
                };
                ensure!(
                    required.iter().all(|p| present.contains(*p)),
                    "incomplete {name} anatomy"
                );
                if name == "Switch.Root" {
                    ensure!(
                        props.contains_key("aria-label") || props.contains_key("aria-labelledby"),
                        "switch needs an accessible name"
                    );
                }
                if name == "Form" {
                    ensure!(
                        props.contains_key("onSubmit") || props.contains_key("action"),
                        "form needs an explicit submission contract"
                    );
                }
            }
            for class in classes {
                ensure!(identifier(class), "invalid CSS Module class");
                ensure!(
                    c.styles.iter().any(|r| regex::Regex::new(&format!(
                        r"\.{}(?:[^a-zA-Z0-9_-]|$)",
                        regex::escape(class)
                    ))
                    .unwrap()
                    .is_match(&r.selector)),
                    "class {class} has no component style"
                );
            }
            for (k, v) in props {
                prop(v, k, env, c, s)?;
            }
            for child in children {
                node(child, env, c, s, count, depth + 1, edges)?;
            }
        }
    }
    Ok(())
}
fn css_value(value: &str, s: &Specification) -> Result<()> {
    ensure!(
        value.len() <= 4096
            && !value.contains([';', '{', '}', '<', '>', '\\', '\n', '\r'])
            && !value.contains("/*")
            && !value.to_lowercase().contains("url(")
            && !value.contains('@'),
        "unsupported CSS value"
    );
    let re = regex::Regex::new(r"var\(\s*--([a-z0-9-]+)").unwrap();
    for c in re.captures_iter(value) {
        ensure!(
            s.tokens.contains_key(&c[1])
                || matches!(
                    &c[1],
                    "positioner-width"
                        | "positioner-height"
                        | "popup-width"
                        | "popup-height"
                        | "available-width"
                        | "available-height"
                        | "transform-origin"
                ),
            "unknown token {}",
            &c[1]
        );
    }
    Ok(())
}
fn rules(rules: &[Rule], s: &Specification, local: bool) -> Result<()> {
    ensure!(rules.len() <= 256, "style rule budget exceeded");
    for r in rules {
        ensure!(
            r.selector.len() <= 256
                && !r.selector.contains(['{', '}', ';', '<', '>', '\\'])
                && !r.selector.contains(":global")
                && !r.selector.contains('@'),
            "unsupported CSS selector"
        );
        ensure!(
            !local || r.selector.split(',').all(|v| v.trim().starts_with('.')),
            "component styles must start with a local class"
        );
        if let Some(m) = &r.media {
            ensure!(
                m.len() <= 256 && !m.contains(['{', '}', ';', '<', '>', '\\', '@']),
                "invalid media condition"
            );
        }
        for (k, v) in &r.declarations {
            ensure!(
                token(k)
                    || k.strip_prefix("-webkit-").is_some_and(token)
                    || k.strip_prefix("-moz-").is_some_and(token),
                "invalid CSS property {k}"
            );
            css_value(v, s)?;
        }
    }
    Ok(())
}
fn visit(
    name: &str,
    edges: &BTreeMap<String, BTreeSet<String>>,
    active: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
) -> Result<()> {
    if done.contains(name) {
        return Ok(());
    }
    ensure!(active.insert(name.into()), "component cycle at {name}");
    for child in &edges[name] {
        visit(child, edges, active, done)?;
    }
    active.remove(name);
    done.insert(name.into());
    Ok(())
}
pub(super) fn specification(s: &Specification, model: &project::Project) -> Result<()> {
    ensure!(
        s.schema_version == 1 && s.input_sha256 == model.input_sha256,
        "stale authored evidence or unsupported schema"
    );
    ensure!(
        !s.routes.is_empty()
            && s.routes.len() <= 128
            && !s.components.is_empty()
            && s.components.len() <= 128
            && s.assets.len() <= 256
            && s.tokens.len() <= 256
            && s.fonts.len() <= 64
            && s.token_scopes.len() <= 32,
        "authored project budget exceeded"
    );
    for (k, url) in &s.assets {
        ensure!(identifier(k), "invalid asset name");
        ensure!(
            model
                .assets
                .get(url)
                .is_some_and(|a| a.status == "local_candidate"),
            "authored asset is not captured locally: {url}"
        );
    }
    for (k, v) in &s.tokens {
        ensure!(token(k), "invalid token name");
        css_value(v, s)?;
    }
    for scope in &s.token_scopes {
        ensure!(
            scope.selector == ":root"
                || scope.selector.starts_with(":root[data-") && scope.selector.ends_with(']'),
            "tokens require root/theme scope so portals inherit them"
        );
        for (k, v) in &scope.values {
            ensure!(s.tokens.contains_key(k), "unknown scoped token");
            css_value(v, s)?;
        }
        rules(
            &[Rule {
                selector: scope.selector.clone(),
                declarations: BTreeMap::new(),
                media: scope.media.clone(),
            }],
            s,
            false,
        )?;
    }
    // Resolve variable dependencies independently in every declared scope.
    let refs = regex::Regex::new(r"var\(\s*--([a-z0-9-]+)").unwrap();
    for override_values in
        std::iter::once(BTreeMap::new()).chain(s.token_scopes.iter().map(|s| s.values.clone()))
    {
        let mut values = s.tokens.clone();
        values.extend(override_values);
        let edges = values
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    refs.captures_iter(v)
                        .filter(|c| s.tokens.contains_key(&c[1]))
                        .map(|c| c[1].to_owned())
                        .collect(),
                )
            })
            .collect();
        for k in values.keys() {
            visit(k, &edges, &mut BTreeSet::new(), &mut BTreeSet::new())?;
        }
    }
    for font in &s.fonts {
        ensure!(s.assets.contains_key(&font.asset), "unknown font asset");
        for v in [&font.family, &font.weight, &font.style, &font.display] {
            css_value(v, s)?;
        }
        ensure!(
            matches!(
                font.display.as_str(),
                "auto" | "block" | "swap" | "fallback" | "optional"
            ),
            "invalid font display"
        );
        if let Some(v) = &font.unicode_range {
            css_value(v, s)?;
        }
    }
    rules(&s.globals, s, false)?;
    let mut count = 0;
    let mut edges = BTreeMap::new();
    let mut paths = BTreeSet::new();
    for (name, c) in &s.components {
        ensure!(
            identifier(name)
                && name.starts_with(char::is_uppercase)
                && !name.starts_with("Component")
                && !name.starts_with("Client")
                && !matches!(
                    name.as_str(),
                    "Fragment"
                        | "Link"
                        | "NavigationMenu"
                        | "Menu"
                        | "Dialog"
                        | "Field"
                        | "Switch"
                        | "Tabs"
                        | "Form"
                        | "Input"
                        | "Button"
                ),
            "component needs a meaningful PascalCase name"
        );
        ensure!(
            c.group.as_ref().is_none_or(|g| token(g)),
            "invalid component group"
        );
        ensure!(
            token(&c.feature) && !c.purpose.trim().is_empty() && !c.evidence.is_empty(),
            "component needs ownership, purpose and provenance"
        );
        ensure!(
            paths.insert(super::emit::component_path(name, c).to_lowercase()),
            "component path collision"
        );
        for source in &c.evidence {
            ensure!(
                model
                    .observations
                    .get(&source.observation)
                    .is_some_and(|o| source.node < o.capture.nodes.len()),
                "invalid component evidence reference"
            );
        }
        shape(
            &Type::Object {
                fields: c.props.clone(),
            },
            0,
        )?;
        ensure!(c.state.len() <= 16, "state budget exceeded");
        for k in c.props.keys() {
            ensure!(!k.starts_with("prop_"), "mechanical prop name refused");
        }
        let mut env = BTreeMap::from([(
            "props".into(),
            Type::Object {
                fields: c.props.clone(),
            },
        )]);
        for (k, v) in &c.state {
            ensure!(
                identifier(k)
                    && !env.contains_key(k)
                    && !reserved_binding(k, s)
                    && !c
                        .state
                        .contains_key(&format!("set{}{}", k[..1].to_uppercase(), &k[1..]))
                    && matches!(v.value_type, Type::Text | Type::Boolean | Type::Number),
                "invalid state definition"
            );
            data(&v.value_type, &v.initial, &s.assets)?;
            env.insert(k.clone(), v.value_type.clone());
        }
        ensure!(c.behaviors.len() <= 8, "behavior budget exceeded");
        ensure!(
            c.behaviors.is_empty()
                || matches!(&c.body,Node::Element { tag, .. } if !matches!(tag.as_str(),"svg"|"path"|"circle"|"line"|"g"|"img"|"br"|"hr"|"title")),
            "behavior requires an HTML container root"
        );
        let mut targets = BTreeSet::new();
        for behavior in &c.behaviors {
            match behavior {
                Behavior::ScrollHeader { minimum_width } => {
                    ensure!(
                        matches!(&c.body,Node::Element { tag, .. } if tag=="header")
                            && *minimum_width <= 8192
                            && targets.insert("@header".to_owned()),
                        "invalid scroll header behavior"
                    );
                }
                Behavior::Reveal {
                    class,
                    effect: _,
                    distance,
                    duration,
                    stagger,
                    threshold,
                } => {
                    ensure!(
                        identifier(class)
                            && has_class(&c.body, class)
                            && targets.insert(class.clone()),
                        "reveal requires a unique owned class target"
                    );
                    ensure!(
                        distance.is_finite()
                            && (0.0..=100.0).contains(distance)
                            && (1..=10000).contains(duration)
                            && *stagger <= 1000
                            && threshold.is_finite()
                            && (0.0..=1.0).contains(threshold),
                        "invalid reveal bounds"
                    );
                }
            }
        }
        rules(&c.styles, s, true)?;
        let mut uses = BTreeSet::new();
        node(&c.body, &env, c, s, &mut count, 0, &mut uses)?;
        edges.insert(name.clone(), uses);
    }
    for (name, entry) in &s.external_components {
        ensure!(
            identifier(name)
                && name.starts_with(char::is_uppercase)
                && !s.components.contains_key(name)
                && !reserved_runtime_binding(name),
            "owned component name collides or is invalid"
        );
        shape(
            &Type::Object {
                fields: entry.props.clone(),
            },
            0,
        )?;
    }
    let mut content_files = BTreeSet::new();
    let mut expanded = 0;
    let mut reachable = BTreeSet::new();
    let mut routes = BTreeSet::new();
    let literal_route = regex::Regex::new(r"^/(?:[A-Za-z0-9_.-]+/?)*$").unwrap();
    for r in &s.routes {
        ensure!(
            model
                .routes
                .values()
                .any(|v| v.pathname == r.path && v.query.is_none()),
            "authored route must be a captured literal path without query"
        );
        ensure!(
            routes.insert(r.path.trim_end_matches('/').to_lowercase()),
            "authored route collision"
        );
        ensure!(
            literal_route.is_match(&r.path)
                && !r.path.contains("//")
                && !r.path.split('/').any(|v| matches!(v, "." | "..")),
            "unsupported authored literal route"
        );
        let c = s
            .components
            .get(&r.component)
            .context("route component missing")?;
        let content = super::emit::content_name(r);
        ensure!(
            token(&content) && content_files.insert(format!("{}/{}", c.feature, content)),
            "duplicate or invalid content filename; provide data_name"
        );
        data(
            &Type::Object {
                fields: c.props.clone(),
            },
            &serde_json::to_value(&r.props)?,
            &s.assets,
        )?;
        visit(&r.component, &edges, &mut BTreeSet::new(), &mut reachable)?;
        let mut values = BTreeMap::from([("props".into(), serde_json::to_value(&r.props)?)]);
        values.extend(c.state.iter().map(|(k, v)| (k.clone(), v.initial.clone())));
        instantiate(&c.body, &values, s, &mut expanded, 0)?;
    }
    ensure!(
        reachable.len() == s.components.len(),
        "unused authored components are refused"
    );
    Ok(())
}

// Check actual data at every component boundary; typed lists alone do not prove
// stable React keys. Bound expansion to refuse pathological nested repetitions.
fn value(expr: &Expression, env: &BTreeMap<String, Value>) -> Result<Value> {
    match expr {
        Expression::Object { fields } => Ok(Value::Object(
            fields
                .iter()
                .map(|(k, e)| Ok((k.clone(), value(e, env)?)))
                .collect::<Result<_>>()?,
        )),
        Expression::List { items } => Ok(Value::Array(
            items.iter().map(|e| value(e, env)).collect::<Result<_>>()?,
        )),

        Expression::Preference { value } if !matches!(value, Preference::SetAccessibilityMode) => {
            Ok(Value::Bool(false))
        }
        Expression::Literal { value } => Ok(value.clone()),
        Expression::Asset { name } => Ok(Value::String(name.clone())),
        Expression::Ref { path } => {
            let mut parts = path.split('.');
            let mut current = env
                .get(parts.next().unwrap())
                .context("missing runtime binding")?;
            for part in parts {
                current = current.get(part).context("missing runtime field")?;
            }
            Ok(current.clone())
        }
        _ => bail!("handler in typed data"),
    }
}
fn instantiate(
    n: &Node,
    env: &BTreeMap<String, Value>,
    s: &Specification,
    count: &mut usize,
    depth: usize,
) -> Result<()> {
    *count += 1;
    ensure!(
        *count <= 100_000 && depth <= 128,
        "expanded component budget exceeded"
    );
    match n {
        Node::If {
            then, otherwise, ..
        } => {
            instantiate(then, env, s, count, depth + 1)?;
            instantiate(otherwise, env, s, count, depth + 1)?;
        }
        Node::Component { name, props } => {
            let props = props
                .iter()
                .map(|(k, e)| Ok((k.clone(), value(e, env)?)))
                .collect::<Result<serde_json::Map<_, _>>>()?;
            let c = &s.components[name];
            let mut child = BTreeMap::from([("props".into(), Value::Object(props))]);
            child.extend(c.state.iter().map(|(k, v)| (k.clone(), v.initial.clone())));
            instantiate(&c.body, &child, s, count, depth + 1)?;
        }
        Node::Repeat {
            each,
            binding,
            key,
            body,
        } => {
            let values = value(each, env)?;
            let mut keys = BTreeSet::new();
            for item in values.as_array().context("repeat value is not a list")? {
                let mut child = env.clone();
                child.insert(binding.clone(), item.clone());
                ensure!(
                    keys.insert(value(key, &child)?.to_string()),
                    "duplicate repeat key"
                );
                instantiate(body, &child, s, count, depth + 1)?;
            }
        }
        Node::Element { children, .. } | Node::Primitive { children, .. } => {
            for n in children {
                instantiate(n, env, s, count, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn has_class(n: &Node, target: &str) -> bool {
    match n {
        Node::Element {
            classes, children, ..
        }
        | Node::Primitive {
            classes, children, ..
        } => classes.iter().any(|c| c == target) || children.iter().any(|n| has_class(n, target)),
        Node::Repeat { body, .. } => has_class(body, target),
        Node::If {
            then, otherwise, ..
        } => has_class(then, target) || has_class(otherwise, target),
        _ => false,
    }
}
