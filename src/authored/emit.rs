use super::*;
use crate::export::structure;
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet};
fn js<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("serializable authored value")
}
pub(super) fn kebab(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('-')
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}
pub(super) fn component_path(name: &str, c: &Component) -> String {
    let base = match c.feature.as_str() {
        "ui" | "compositions" => "src/components/compositions".to_owned(),
        "layout" => "src/components/layout".to_owned(),
        feature => format!("src/features/{feature}"),
    };
    let group = c
        .group
        .as_ref()
        .map(|g| format!("/{g}"))
        .unwrap_or_default();
    format!("{base}{group}/{}.tsx", kebab(name))
}
pub(super) fn content_name(route: &Route) -> String {
    route.data_name.clone().unwrap_or_else(|| {
        format!(
            "{}-content",
            kebab(
                route
                    .component
                    .strip_suffix("Page")
                    .unwrap_or(&route.component)
            )
        )
    })
}
fn typename(t: &Type) -> String {
    match t {
        Type::Text | Type::Href | Type::Asset => "string".into(),
        Type::Number => "number".into(),
        Type::Boolean => "boolean".into(),
        Type::List { item } => format!("Array<{}>", typename(item)),
        Type::Object { fields } => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(k, v)| format!("{k}:{};", typename(v)))
                .collect::<String>()
        ),
    }
}
fn setter(name: &str) -> String {
    format!("set{}{}", name[..1].to_uppercase(), &name[1..])
}
fn expression(e: &Expression) -> String {
    match e {
        Expression::Assign { state, value } => {
            format!("() => {}({})", setter(state), expression(value))
        }
        Expression::Object { fields } => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(k, v)| format!("{}:{}", js(k), expression(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Expression::List { items } => format!(
            "[{}]",
            items.iter().map(expression).collect::<Vec<_>>().join(",")
        ),
        Expression::Preference { value } => format!(
            "motion.{}",
            match value {
                Preference::AccessibilityMode => "accessibilityMode",
                Preference::ReducedMotion => "reducedMotion",
                Preference::SetAccessibilityMode => "setAccessibilityMode",
            }
        ),
        Expression::Literal { value } => js(value),
        Expression::Ref { path } => path.clone(),
        Expression::Asset { name } => format!("assets.{name}"),
        Expression::Setter { state } => setter(state),
        Expression::Set {
            state,
            value,
            prevent_default,
        } => {
            if *prevent_default {
                format!(
                    "(event) => {{ event.preventDefault(); {}({}); }}",
                    setter(state),
                    js(value)
                )
            } else {
                format!("() => {}({})", setter(state), js(value))
            }
        }
        Expression::Link { href } => format!("<Link to={{{}}} />", expression(href)),
    }
}
fn jsx_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('{', "&#123;")
        .replace('}', "&#125;")
}
fn node(n: &Node, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    match n {
        Node::If {
            condition,
            then,
            otherwise,
        } => format!(
            "{{{} ? <>\n{indent}  {}\n{indent}</> : <>\n{indent}  {}\n{indent}</>}}",
            expression(condition),
            node(then, depth + 1),
            node(otherwise, depth + 1)
        ),
        Node::Text {
            value:
                Expression::Literal {
                    value: serde_json::Value::String(value),
                },
        } if !value.contains(['\n', '\r']) && value.trim() == value => jsx_text(value),
        Node::Text { value } => format!("{{{}}}", expression(value)),
        Node::Component { name, props } | Node::External { name, props } => {
            format!("<{name}{} />", properties(props, &[]))
        }
        Node::Element {
            tag,
            props,
            classes,
            children,
        }
        | Node::Primitive {
            name: tag,
            props,
            classes,
            children,
        } => {
            let p = properties(props, classes);
            if children.is_empty() {
                format!("<{tag}{p} />")
            } else if children.iter().all(|n| matches!(n, Node::Text { .. })) {
                format!(
                    "<{tag}{p}>{}</{tag}>",
                    children
                        .iter()
                        .map(|n| node(n, depth + 1))
                        .collect::<String>()
                )
            } else {
                format!(
                    "<{tag}{p}>\n{}\n{indent}</{tag}>",
                    children
                        .iter()
                        .map(|n| format!("{indent}  {}", node(n, depth + 1)))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
        }
        Node::Repeat {
            each,
            binding,
            key,
            body,
        } => format!(
            "{{{}.map(({binding}) => (\n{indent}  <Fragment key={{{}}}>\n{indent}    {}\n{indent}  </Fragment>\n{indent}))}}",
            expression(each),
            expression(key),
            node(body, depth + 2)
        ),
    }
}
fn properties(props: &BTreeMap<String, Expression>, classes: &[String]) -> String {
    let mut out = props
        .iter()
        .map(|(k, v)| match v {
            Expression::Literal {
                value: serde_json::Value::String(v),
            } if !v.contains(['\n', '\r']) => {
                format!("{k}=\"{}\"", jsx_text(v).replace('"', "&quot;"))
            }
            Expression::Literal {
                value: serde_json::Value::Bool(true),
            } => k.clone(),
            _ => format!("{k}={{{}}}", expression(v)),
        })
        .collect::<Vec<_>>();
    if let [class] = classes {
        out.push(format!("className={{styles.{class}}}"));
    } else if !classes.is_empty() {
        out.push(format!(
            "className={{[{}].join(' ')}}",
            classes
                .iter()
                .map(|c| format!("styles.{c}"))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if out.is_empty() {
        String::new()
    } else {
        format!(" {}", out.join(" "))
    }
}
fn dependencies(n: &Node, components: &mut BTreeSet<String>, primitives: &mut BTreeSet<String>) {
    match n {
        Node::If {
            then, otherwise, ..
        } => {
            dependencies(then, components, primitives);
            dependencies(otherwise, components, primitives);
        }
        Node::Component { name, .. } | Node::External { name, .. } => {
            components.insert(name.clone());
        }
        Node::Primitive { name, children, .. } => {
            primitives.insert(name.split('.').next().unwrap().into());
            for child in children {
                dependencies(child, components, primitives);
            }
        }
        Node::Element { children, .. } => {
            for child in children {
                dependencies(child, components, primitives);
            }
        }
        Node::Repeat { body, .. } => dependencies(body, components, primitives),
        _ => {}
    }
}
#[derive(Default)]
struct Usage {
    preference: bool,
    repeat: bool,
    link: bool,
    assets: bool,
    styles: bool,
}
impl Usage {
    fn expression(&mut self, e: &Expression) {
        match e {
            Expression::Object { fields } => {
                for e in fields.values() {
                    self.expression(e);
                }
            }
            Expression::List { items } => {
                for e in items {
                    self.expression(e);
                }
            }
            Expression::Assign { value, .. } => self.expression(value),
            Expression::Preference { .. } => self.preference = true,
            Expression::Asset { .. } => self.assets = true,
            Expression::Link { href } => {
                self.link = true;
                self.expression(href);
            }
            _ => {}
        }
    }
    fn node(&mut self, n: &Node) {
        match n {
            Node::If {
                condition,
                then,
                otherwise,
            } => {
                self.expression(condition);
                self.node(then);
                self.node(otherwise);
            }
            Node::Text { value } => self.expression(value),
            Node::Component { props, .. } | Node::External { props, .. } => {
                for e in props.values() {
                    self.expression(e);
                }
            }
            Node::Element {
                props,
                classes,
                children,
                ..
            }
            | Node::Primitive {
                props,
                classes,
                children,
                ..
            } => {
                self.styles |= !classes.is_empty();
                for e in props.values() {
                    self.expression(e);
                }
                for n in children {
                    self.node(n);
                }
            }
            Node::Repeat {
                each, key, body, ..
            } => {
                self.repeat = true;
                self.expression(each);
                self.expression(key);
                self.node(body);
            }
        }
    }
}
fn localize_node(n: &mut Node, s: &Specification) {
    match n {
        Node::If {
            then, otherwise, ..
        } => {
            localize_node(then, s);
            localize_node(otherwise, s);
        }
        Node::Component { name, props } | Node::External { name, props } => {
            let types = if let Some(c) = s.components.get(name) {
                &c.props
            } else {
                &s.external_components[name].props
            };
            for (key, expr) in props {
                if let Expression::Literal { value } = expr {
                    *expr = data_expression(&types[key], value);
                }
            }
        }
        Node::Element {
            props, children, ..
        }
        | Node::Primitive {
            props, children, ..
        } => {
            for (key, expr) in props {
                if matches!(key.as_str(), "src" | "poster")
                    && let Expression::Literal { value } = expr
                {
                    *expr = data_expression(&Type::Asset, value);
                }
            }
            for child in children {
                localize_node(child, s);
            }
        }
        Node::Repeat { body, .. } => localize_node(body, s),
        _ => {}
    }
}
fn component(name: &str, c: &Component, s: &Specification) -> String {
    let mut components = BTreeSet::new();
    let mut primitives = BTreeSet::new();
    dependencies(&c.body, &mut components, &mut primitives);
    let mut usage = Usage::default();
    usage.node(&c.body);
    let mut text = String::new();
    let mut react = Vec::new();
    if usage.repeat {
        react.push("Fragment");
    }
    if !c.state.is_empty() {
        react.push("useState");
    }
    if !react.is_empty() {
        text.push_str(&format!("import {{{}}} from 'react'\n", react.join(",")));
    }
    if !c.behaviors.is_empty() {
        text.push_str("import {useRef, type ComponentRef} from 'react'\n");
    }
    if usage.preference {
        text.push_str(
            "import {useMotionPreference} from '@/components/providers/motion-preference'\n",
        );
    }
    if c.behaviors
        .iter()
        .any(|b| matches!(b, Behavior::ScrollHeader { .. }))
    {
        text.push_str("import {useScrollHeader} from '@/hooks/use-scroll-header'\n");
    }
    if c.behaviors
        .iter()
        .any(|b| matches!(b, Behavior::Reveal { .. }))
    {
        text.push_str("import {useScrollReveal} from '@/hooks/use-scroll-reveal'\n");
    }
    if usage.link {
        text.push_str("import {Link} from '@tanstack/react-router'\n");
    }
    if usage.assets {
        text.push_str("import {assets} from '@/lib/assets'\n");
    }
    if usage.styles {
        text.push_str(&format!(
            "import styles from './{}.module.css'\n",
            kebab(name)
        ));
    }
    for group in primitives {
        let path =
            super::validate::primitive(if matches!(group.as_str(), "Input" | "Form" | "Button") {
                group.as_str()
            } else {
                ""
            });
        let path = path.map(str::to_owned).unwrap_or_else(|| kebab(&group));
        text.push_str(&format!(
            "import {{{group}}} from '@/components/ui/{path}'\n"
        ));
    }
    for dependency in components {
        let path = if let Some(c) = s.components.get(&dependency) {
            component_path(&dependency, c)
        } else {
            format!("src/{}", s.external_components[&dependency].module)
        };
        text.push_str(&format!(
            "import {{{dependency}}} from '@/{}'\n",
            path.strip_prefix("src/").unwrap().trim_end_matches(".tsx")
        ));
    }
    // Evidence is in the manifest rather than noisy inline source comments.
    text.push_str(&format!(
        "export type {name}Props={}\nexport function {name}({}){{\n",
        typename(&Type::Object {
            fields: c.props.clone()
        }),
        if c.props.is_empty() {
            String::new()
        } else {
            format!("props:{name}Props")
        }
    ));
    if usage.preference {
        text.push_str("const motion = useMotionPreference();\n");
    }
    let mut body = c.body.clone();
    if let Node::Element { tag, props, .. } = &mut body
        && !c.behaviors.is_empty()
    {
        text.push_str(&format!(
            "const scope = useRef<ComponentRef<{}>>(null);\n",
            js(tag)
        ));
        props.insert(
            "ref".into(),
            Expression::Ref {
                path: "scope".into(),
            },
        );
        for behavior in &c.behaviors {
            match behavior {
                Behavior::ScrollHeader { minimum_width } => text.push_str(&format!("useScrollHeader(scope,{minimum_width});\n")),
                Behavior::Reveal { class, effect, distance, duration, stagger, threshold } => text.push_str(&format!("useScrollReveal(scope,styles.{class},{});\n",js(&serde_json::json!({"effect":effect,"distance":distance,"duration":duration,"stagger":stagger,"threshold":threshold})))),
            }
        }
    }
    for (k, v) in &c.state {
        text.push_str(&format!(
            "const [{k},{}]=useState<{}>({});\n",
            setter(k),
            typename(&v.value_type),
            js(&v.initial)
        ));
    }
    text.push_str(&format!(
        "return (\n  <>\n    {}\n  </>\n);\n}}\n",
        node(&body, 2)
    ));
    text
}
fn css(rules: &[Rule]) -> String {
    rules
        .iter()
        .map(|r| {
            let body = format!(
                "{} {{\n{}\n}}\n",
                r.selector,
                r.declarations
                    .iter()
                    .map(|(k, v)| format!("  {k}: {v};"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            r.media
                .as_ref()
                .map(|m| format!("@media {m} {{\n{body}}}\n"))
                .unwrap_or(body)
        })
        .collect()
}
fn component_css(rules: &[Rule]) -> String {
    let mut rules = rules.to_vec();
    for rule in &mut rules {
        let mut utilities = Vec::new();
        for (property, value, utility) in [
            ("display", "flex", "flex"),
            ("display", "grid", "grid"),
            ("display", "inline-flex", "inline-flex"),
            ("display", "block", "block"),
            ("align-items", "center", "items-center"),
            ("justify-content", "center", "justify-center"),
            ("justify-content", "space-between", "justify-between"),
            ("flex-direction", "column", "flex-col"),
            ("position", "relative", "relative"),
            ("position", "absolute", "absolute"),
            ("text-align", "center", "text-center"),
            ("overflow", "hidden", "overflow-hidden"),
            ("width", "100%", "w-full"),
        ] {
            if rule.declarations.get(property).is_some_and(|v| v == value) {
                rule.declarations.remove(property);
                utilities.push(utility);
            }
        }
        if !utilities.is_empty() {
            rule.declarations
                .insert("@apply".into(), utilities.join(" "));
        }
    }
    format!(
        "@reference \"@/styles/tailwind.css\";\n{}",
        css(&rules).replace("@apply:", "@apply")
    )
}
fn data_expression(t: &Type, v: &serde_json::Value) -> Expression {
    match (t, v) {
        (Type::Asset, serde_json::Value::String(name)) => Expression::Asset { name: name.clone() },
        (Type::List { item }, serde_json::Value::Array(values)) => Expression::List {
            items: values.iter().map(|v| data_expression(item, v)).collect(),
        },
        (Type::Object { fields }, serde_json::Value::Object(values)) => Expression::Object {
            fields: fields
                .iter()
                .map(|(k, t)| (k.clone(), data_expression(t, &values[k])))
                .collect(),
        },
        _ => Expression::Literal { value: v.clone() },
    }
}
const ROOT: &str = r#"import {createRootRoute, HeadContent, Outlet, Scripts} from '@tanstack/react-router'
import '@/styles/tailwind.css'
import tokens from '@/styles/tokens.css?url'
import fonts from '@/styles/fonts.css?url'
import globals from '@/styles/globals.css?url'
export const Route=createRootRoute({head:()=>({meta:[{charSet:'utf-8'},{name:'viewport',content:'width=device-width, initial-scale=1'}],links:[{rel:'stylesheet',href:tokens},{rel:'stylesheet',href:fonts},{rel:'stylesheet',href:globals}]}),component:Document,notFoundComponent:()=> <main><h1>Page not reconstructed</h1></main>})
function Document(){return <html lang="en"><head><HeadContent /></head><body className="relative"><div className="isolate"><Outlet /></div><Scripts /></body></html>}
"#;
const ROUTER: &str = r#"import {createRouter} from '@tanstack/react-router'
import {routeTree} from './routeTree.gen'
export function getRouter(){return createRouter({routeTree,scrollRestoration:true,caseSensitive:true,trailingSlash:'preserve'})}
declare module '@tanstack/react-router'{interface Register{router:ReturnType<typeof getRouter>}}
"#;
pub(super) fn project(
    root: &Path,
    out: &Path,
    s: &Specification,
    model: &project::Project,
    specification_sha256: &str,
    sources: &super::source::Files,
) -> Result<Report> {
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    for &(path, text) in crate::export::TEMPLATE {
        if !path.starts_with("src/") {
            files.insert(path.into(), text.into());
        }
    }
    files.insert(
        "src/styles/tailwind.css".into(),
        include_str!("../export_template/src/styles/tailwind.css").replace(
            "@layer theme, base, components, utilities;",
            "@layer theme, base, components, utilities;\n@import 'tailwindcss/preflight.css' layer(base);",
        ),
    );
    let mut used_primitives = BTreeSet::new();
    for c in s.components.values() {
        dependencies(&c.body, &mut BTreeSet::new(), &mut used_primitives);
    }
    for (name, path) in [
        ("NavigationMenu", "navigation-menu"),
        ("Menu", "menu"),
        ("Dialog", "dialog"),
        ("Field", "field"),
        ("Form", "form"),
        ("Input", "input"),
        ("Button", "button"),
        ("Switch", "switch"),
        ("Tabs", "tabs"),
    ] {
        if !used_primitives.contains(name)
            && !sources
                .values()
                .any(|source| source.contains(&format!("components/ui/{path}")))
        {
            continue;
        }
        files.insert(
            format!("src/components/ui/{path}.tsx"),
            format!("export {{ {name} }} from '@base-ui/react/{path}';\n"),
        );
    }
    let mut root_source = ROOT.to_owned();
    let uses_motion = sources
        .values()
        .any(|s| s.contains("@/components/providers/motion-preference"))
        || s.components.values().any(|c| {
            let mut usage = Usage::default();
            usage.node(&c.body);
            usage.preference || !c.behaviors.is_empty()
        });
    if uses_motion {
        root_source.insert_str(
            0,
            "import {MotionPreferenceProvider} from '@/components/providers/motion-preference'\n",
        );
        root_source = root_source.replace(
            "<Outlet />",
            "<MotionPreferenceProvider><Outlet /></MotionPreferenceProvider>",
        );
        files.insert(
            "src/components/providers/motion-preference.tsx".into(),
            include_str!("template/motion-preference.tsx").into(),
        );
    }
    if s.components.values().any(|c| {
        c.behaviors
            .iter()
            .any(|b| matches!(b, Behavior::ScrollHeader { .. }))
    }) {
        files.insert(
            "src/hooks/use-scroll-header.ts".into(),
            include_str!("template/use-scroll-header.ts").into(),
        );
    }
    if s.components.values().any(|c| {
        c.behaviors
            .iter()
            .any(|b| matches!(b, Behavior::Reveal { .. }))
    }) {
        files.insert(
            "src/hooks/use-scroll-reveal.ts".into(),
            include_str!("template/use-scroll-reveal.ts").into(),
        );
    }
    files.insert("src/routes/__root.tsx".into(), root_source);
    files.insert("src/router.tsx".into(), ROUTER.into());
    let mut assets = BTreeMap::new();
    let mut total_assets = 0;
    for (name, url) in &s.assets {
        let asset = &model.assets[url];
        let relative = asset.local_path.as_ref().context("asset path missing")?;
        let path = util::checked_path(root, &root.join(relative))?;
        let bytes = super::read(&path, 16 * 1024 * 1024)?;
        total_assets += bytes.len();
        ensure!(
            total_assets <= 256 * 1024 * 1024,
            "authored asset byte budget exceeded"
        );
        ensure!(
            Some(&fingerprint(&bytes)) == asset.local_sha256.as_ref(),
            "authored asset bytes changed"
        );
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        ensure!(
            matches!(
                extension.as_str(),
                "woff"
                    | "woff2"
                    | "ttf"
                    | "otf"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "webp"
                    | "avif"
                    | "gif"
                    | "svg"
                    | "ico"
            ),
            "unsupported authored asset format"
        );
        let category = if matches!(extension.as_str(), "woff" | "woff2" | "ttf" | "otf") {
            "fonts"
        } else {
            "images"
        };
        let target = format!("assets/{category}/{}.{}", kebab(name), extension);
        ensure!(
            !assets
                .values()
                .any(|v: &String| v.eq_ignore_ascii_case(&target)),
            "asset filename collision"
        );
        util::write(out, &out.join(format!("src/{target}")), &bytes)?;
        assets.insert(name.clone(), target);
    }
    files.insert(
        "src/lib/assets.ts".into(),
        format!(
            "{}\nexport const assets={{{}}} as const;\n",
            assets
                .iter()
                .map(|(name, path)| format!("import {name}Url from '@/{path}';"))
                .collect::<Vec<_>>()
                .join("\n"),
            assets
                .keys()
                .map(|name| format!("{name}:{name}Url"))
                .collect::<Vec<_>>()
                .join(",")
        ),
    );
    let mut tokens = String::from(
        "/* Reference and semantic tokens. Keep scoped overrides with their context. */\n:root {\n",
    );
    for (k, v) in &s.tokens {
        tokens.push_str(&format!("  --{k}: {v};\n"));
    }
    tokens.push_str("}\n");
    for scope in &s.token_scopes {
        tokens.push_str(&css(&[Rule {
            selector: scope.selector.clone(),
            declarations: scope
                .values
                .iter()
                .map(|(k, v)| (format!("--{k}"), v.clone()))
                .collect(),
            media: scope.media.clone(),
        }]));
    }
    files.insert("src/styles/tokens.css".into(), tokens);
    let mut fonts = String::new();
    for f in &s.fonts {
        let extension = Path::new(&assets[&f.asset])
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        ensure!(
            matches!(extension, "woff" | "woff2" | "ttf" | "otf"),
            "font requires an actual font asset"
        );
        fonts.push_str(&format!("@font-face {{\n  font-family: {};\n  src: url({});\n  font-weight: {};\n  font-style: {};\n  font-display: {};\n",js(&f.family),js(&format!("../{}",assets[&f.asset])),f.weight,f.style,f.display));
        if let Some(range) = &f.unicode_range {
            fonts.push_str(&format!("  unicode-range: {range};\n"));
        }
        fonts.push_str("}\n");
    }
    files.insert("src/styles/fonts.css".into(), fonts);
    files.insert("src/styles/globals.css".into(), css(&s.globals));
    let mut component_paths = BTreeMap::new();
    for (name, c) in &s.components {
        let path = component_path(name, c);
        let mut localized = c.clone();
        localize_node(&mut localized.body, s);
        files.insert(path.clone(), component(name, &localized, s));
        if !c.styles.is_empty() {
            files.insert(
                path.replace(".tsx", ".module.css"),
                component_css(&c.styles),
            );
        }
        component_paths.insert(name.clone(), path);
    }
    for route in &s.routes {
        let directory = format!("src/features/{}/", s.components[&route.component].feature);
        let data_name = content_name(route);
        let props = data_expression(
            &Type::Object {
                fields: s.components[&route.component].props.clone(),
            },
            &serde_json::to_value(&route.props)?,
        );
        let mut usage = Usage::default();
        usage.expression(&props);
        let asset_import = if usage.assets {
            "import {assets} from '@/lib/assets'\n"
        } else {
            ""
        };
        let component_import = component_paths[&route.component]
            .trim_start_matches("src/")
            .trim_end_matches(".tsx");
        files.insert(
            format!("{directory}{data_name}.ts"),
            format!(
                "{asset_import}import type {{{}Props}} from '@/{}'\nexport const content:{}Props={}\n",
                route.component,
                component_import,
                route.component,
                expression(&props)
            ),
        );
        let id = format!("{}/", route.path.trim_end_matches('/'));
        files.insert(format!("src/routes/{}",structure::route_file(&route.path)),format!("import {{createFileRoute}} from '@tanstack/react-router'\nimport {{{0}}} from '@/{component_import}'\nimport {{content}} from '@/{1}{data_name}'\nexport const Route=createFileRoute({2})({{head:()=>({{meta:[{{title:{3}}}]}}),component:RouteComponent}})\nfunction RouteComponent(){{return <{0} {{...content}} />}}\n",route.component,directory.trim_start_matches("src/"),js(&id),js(&route.title)));
    }
    for (path, source) in sources {
        ensure!(
            files
                .insert(format!("src/{path}"), source.clone())
                .is_none(),
            "owned source collides with generated file: {path}"
        );
    }
    if s.libraries.iter().any(|l| l == "three") {
        let mut package: serde_json::Value = serde_json::from_str(&files["package.json"])?;
        package["dependencies"]["three"] = "0.183.0".into();
        package["devDependencies"]["@types/three"] = "0.183.0".into();
        files.insert(
            "package.json".into(),
            serde_json::to_string_pretty(&package)?,
        );
        files.insert(
            "bun.lock".into(),
            include_str!("../export_template/bun-graphics.lock").into(),
        );
    }
    let source_modules = sources
        .iter()
        .map(|(path, source)| {
            let path = format!("src/{path}");
            Ok((
                path.clone(),
                fingerprint(structure::format_source(&path, source)?.as_bytes()),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let report = Report {
        schema_version: 1,
        source_quality: "authored_candidate_requires_behavior_and_visual_qualification".into(),
        input_sha256: s.input_sha256.clone(),
        specification_sha256: specification_sha256.into(),
        routes: s.routes.len(),
        components: s.components.len(),
        component_paths,
        recovered_modules: 0,
        source_modules,
        gaps: s.gaps.clone(),
        verification: "not_run".into(),
    };
    files.insert(
        ".mochi/authored-report.json".into(),
        serde_json::to_string_pretty(&report)?,
    );
    files.insert(
        ".mochi/authored-specification.json".into(),
        serde_json::to_string_pretty(s)?,
    );
    files.insert("README.md".into(),"# Authored reconstruction candidate\n\nThis app is generated from a reviewed semantic specification. It contains direct Base UI compositions, typed components, feature data, scoped CSS Modules, semantic tokens, and local fonts. No deployed bundle or recovered controller is imported. This is a candidate, not a visual or behavior qualification.\n\nRun `bun install --frozen-lockfile`, `bun run build`, `bun run check`, `bun start`; use `bun run dev` while editing.\n\nEdit content in `src/features`, reusable compositions in `src/components/compositions`, and design values in `src/styles/tokens.css`. Add routes under `src/routes`. `src/components/ui` contains only transparent Base UI primitive exports. Tailwind v4 provides utilities and semantic theme aliases; scoped CSS handles measured layouts and graphics. Motion owns authored animation effects. Application source and public assets belong in version control; `.mochi`, `.output`, `.tanstack`, and dependencies do not.\n\nSee EDITING.md for the component map and known gaps.\n".into());
    let mut editing =
        String::from("# Source ownership\n\n| Component | File | Purpose |\n| --- | --- | --- |\n");
    for (name, c) in &s.components {
        editing.push_str(&format!(
            "| {name} | `{}` | {} |\n",
            report.component_paths[name],
            c.purpose.replace('|', "/")
        ));
    }
    editing.push_str("\n## Reconstruction gaps\n\n");
    if s.gaps.is_empty() {
        editing.push_str("No gaps declared by the specification author. This is not independent qualification.\n")
    } else {
        for gap in &s.gaps {
            editing.push_str(&format!("- {gap}\n"));
        }
    }
    files.insert("EDITING.md".into(), editing);
    let mut total = 0;
    let mut portable = BTreeSet::new();
    for (path, source) in files {
        ensure!(
            portable.insert(path.to_lowercase()),
            "authored path collision"
        );
        let source = structure::format_source(&path, &source)?;
        total += source.len();
        ensure!(
            total <= 32 * 1024 * 1024,
            "authored source output budget exceeded"
        );
        util::write(out, &out.join(path), source.as_bytes())?;
    }
    Ok(report)
}
