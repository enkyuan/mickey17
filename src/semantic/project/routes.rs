//! Routes: which captured pages become TanStack file routes, under which names, and the route and
//! page source files. Route paths are the captured literal paths; nothing is inferred as dynamic.
use crate::semantic::components::{pascal, words};
use crate::semantic::emit::ts_string_lit;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

#[derive(Debug, Clone, Serialize)]
pub struct RoutePlan {
    /// The route path (`/pay`).
    pub path: String,
    /// File route name without extension (`index`, `pay`, `blog.direct`, `models_.x`).
    pub file: String,
    /// The route id `createFileRoute` takes (`/`, `/pay`, `/models_/x`).
    pub id: String,
    /// Directory segment for this route's components and data (`home`, `blog-direct`).
    pub ns: String,
    /// Page component name (`HomePage`).
    pub component: String,
    pub url: String,
    /// Evidence file relative to the capture directory.
    pub evidence: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteSkip {
    pub url: String,
    pub reason: String,
}

fn viewport_of(evidence: &str) -> Option<(u32, u32)> {
    let stem = evidence.rsplit('/').next()?.strip_suffix(".json")?;
    let (w, h) = stem.rsplit_once('-')?.1.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// The evidence file for a viewport (`1440x900`), else the widest captured.
pub fn pick_evidence(page: &Value, viewport: Option<&str>) -> Option<String> {
    let files: Vec<&str> = page["source_evidence"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    match viewport {
        Some(v) => files
            .into_iter()
            .find(|f| f.ends_with(&format!("-{v}.json")))
            .map(str::to_owned),
        None => files
            .into_iter()
            .max_by_key(|f| viewport_of(f).map(|v| v.0))
            .map(str::to_owned),
    }
}

/// Segments TanStack's file router would treat specially, or that cannot be a file name.
fn segment_ok(segment: &str) -> Result<(), &'static str> {
    if segment.is_empty() {
        return Err("empty path segment");
    }
    if segment.starts_with(['-', '_', '$', '(']) {
        return Err(
            "a segment starting with -, _, $ or ( is a route-file convention, not a captured path",
        );
    }
    if !segment
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '~'))
    {
        return Err(
            "a segment with characters other than letters, digits, - _ ~ cannot be a file route name",
        );
    }
    Ok(())
}

/// Plan the routes of a capture. Pages that differ only by query or fragment, share a path with an
/// earlier page, leave the origin of the first page, or cannot be a file route are skipped, each
/// with its reason.
pub fn plan(
    manifest: &Value,
    viewport: Option<&str>,
    limit: Option<usize>,
) -> (Vec<RoutePlan>, Vec<RouteSkip>) {
    struct Entry {
        path: String,
        segments: Vec<String>,
        url: String,
        evidence: String,
        title: String,
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut skipped = Vec::new();
    let mut origin: Option<String> = None;
    let mut taken: BTreeMap<String, String> = BTreeMap::new();
    let Some(pages) = manifest["pages"].as_array() else {
        return (Vec::new(), skipped);
    };
    for page in pages {
        let raw = page["final_url"]
            .as_str()
            .or(page["url"].as_str())
            .unwrap_or("");
        let skip = |reason: &str| RouteSkip {
            url: raw.to_owned(),
            reason: reason.to_owned(),
        };
        let Ok(url) = Url::parse(raw) else {
            skipped.push(skip("not a URL"));
            continue;
        };
        let this_origin = url.origin().ascii_serialization();
        if *origin.get_or_insert_with(|| this_origin.clone()) != this_origin {
            skipped.push(skip("another origin than the first page"));
            continue;
        }
        let mut path = url.path().to_owned();
        let mut note = String::new();
        if path.len() > 1 && path.ends_with('/') {
            path.pop();
            note = " (trailing slash dropped)".into();
        }
        let segments: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        if let Some(reason) = segments.iter().find_map(|s| segment_ok(s).err()) {
            skipped.push(skip(reason));
            continue;
        }
        if let Some(first) = taken.get(&path) {
            skipped.push(skip(&format!(
                "same path as {first}{note}; query and fragment variants are one route"
            )));
            continue;
        }
        let Some(evidence) = pick_evidence(page, viewport) else {
            skipped.push(skip("no evidence file for the requested viewport"));
            continue;
        };
        if limit.is_some_and(|l| entries.len() >= l) {
            skipped.push(skip("beyond the --routes limit"));
            continue;
        }
        taken.insert(path.clone(), raw.to_owned());
        entries.push(Entry {
            path,
            segments,
            url: raw.to_owned(),
            evidence,
            title: page["title"].as_str().unwrap_or("").to_owned(),
        });
    }
    // Name the files. A route whose path is a prefix of another captured route would become that
    // route's layout in TanStack's flat routing (its page would then need an <Outlet/> and the
    // child would render inside it), so such a segment is written `models_`, which opts out of
    // nesting: both pages stay what the capture shows.
    let paths: BTreeSet<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    let mut plans: Vec<RoutePlan> = Vec::new();
    let mut used_ns: BTreeSet<String> = BTreeSet::new();
    let mut used_file: BTreeSet<String> = BTreeSet::new();
    for e in &entries {
        let (file, ns) = if e.segments.is_empty() {
            ("index".to_owned(), "home".to_owned())
        } else {
            let last = e.segments.len() - 1;
            let file = e
                .segments
                .iter()
                .enumerate()
                .map(|(i, seg)| {
                    let prefix = format!("/{}", e.segments[..=i].join("/"));
                    if i < last && paths.contains(prefix.as_str()) {
                        format!("{seg}_")
                    } else {
                        seg.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(".");
            (file, e.segments.join("-").to_lowercase())
        };
        // Case-insensitive file systems and shared namespaces: a collision is a skipped route,
        // not a silently overwritten page.
        if used_ns.contains(&ns) || used_file.contains(&file.to_lowercase()) {
            skipped.push(RouteSkip {
                url: e.url.clone(),
                reason: format!(
                    "its component directory `{ns}` or route file `{file}` collides with an earlier route (names compared case-insensitively)"
                ),
            });
            continue;
        }
        used_ns.insert(ns.clone());
        used_file.insert(file.to_lowercase());
        let base = pascal(&words(&ns).join(" "));
        // An identifier cannot start with a digit.
        let component = if base.starts_with(|c: char| c.is_ascii_digit()) {
            format!("Route{base}Page")
        } else {
            format!("{base}Page")
        };
        let id = if file == "index" {
            "/".to_owned()
        } else {
            format!("/{}", file.replace('.', "/"))
        };
        plans.push(RoutePlan {
            path: e.path.clone(),
            file,
            id,
            ns,
            component,
            url: e.url.clone(),
            evidence: e.evidence.clone(),
            title: e.title.clone(),
        });
    }
    (plans, skipped)
}

/// The route file: head from the capture, the document attributes, and the page component.
pub struct RouteHead<'a> {
    pub title: &'a str,
    /// `(kind, key, content)` with kind `name` / `property` / `httpEquiv`.
    pub meta: Vec<(String, String, String)>,
    pub links: Vec<Vec<(String, String)>>,
    pub json_ld: Vec<String>,
    pub lang: Option<&'a str>,
    pub html: Vec<(String, String)>,
    pub body: Vec<(String, String)>,
}

fn object(entries: &[(String, String)]) -> String {
    let fields: Vec<String> = entries
        .iter()
        .map(|(k, v)| {
            let key = if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !k.starts_with(|c: char| c.is_ascii_digit())
            {
                k.clone()
            } else {
                ts_string_lit(k)
            };
            format!("{key}: {}", ts_string_lit(v))
        })
        .collect();
    format!("{{ {} }}", fields.join(", "))
}

pub fn route_file(plan: &RoutePlan, head: &RouteHead<'_>) -> String {
    let mut meta: Vec<String> = Vec::new();
    if !head.title.is_empty() {
        meta.push(format!("{{ title: {} }}", ts_string_lit(head.title)));
    }
    for (kind, key, content) in &head.meta {
        meta.push(object(&[
            (kind.clone(), key.clone()),
            ("content".into(), content.clone()),
        ]));
    }
    let links: Vec<String> = head.links.iter().map(|l| object(l)).collect();
    let scripts: Vec<String> = head
        .json_ld
        .iter()
        .map(|j| {
            format!(
                "{{ type: 'application/ld+json', children: {} }}",
                ts_string_lit(j)
            )
        })
        .collect();
    let mut html = head.html.clone();
    if let Some(lang) = head.lang.filter(|l| !l.is_empty())
        && !html.iter().any(|(k, _)| k == "lang")
    {
        html.insert(0, ("lang".into(), lang.to_owned()));
    }
    let list = |items: &[String]| {
        if items.is_empty() {
            "[]".to_owned()
        } else {
            format!("[\n      {},\n    ]", items.join(",\n      "))
        }
    };
    format!(
        "// Generated by `mochi semantic --assemble` for the captured route {path}.\n\
         import {{ createFileRoute }} from '@tanstack/react-router'\n\
         import {{ {component} }} from '@/pages/{ns}'\n\
         import type {{ RouteDocument }} from '@/lib/document'\n\
         \n\
         /** `<html>` and `<body>` attributes as captured at the evidence viewport. */\n\
         const captured: RouteDocument = {{\n  html: {html},\n  body: {body},\n}}\n\
         \n\
         export const Route = createFileRoute({id_lit})({{\n  \
           head: () => ({{\n    meta: {meta},\n    links: {links},\n    scripts: {scripts},\n  }}),\n  \
           loader: () => ({{ document: captured }}),\n  \
           component: {component},\n}})\n",
        path = plan.path,
        id_lit = ts_string_lit(&plan.id),
        component = plan.component,
        ns = plan.ns,
        html = object(&html),
        body = object(&head.body),
        meta = list(&meta),
        links = list(&links),
        scripts = list(&scripts),
    )
}

pub fn page_file(plan: &RoutePlan, jsx: &str, hooks: &[String], imports: &[String]) -> String {
    let mut out = format!(
        "// Generated by `mochi semantic --assemble` from {} (route {}).\n// Base UI compositions and repeated-structure components are rendered in place; the rest of the\n// captured body is plain markup. Where styles come from is stated in the project report.\n",
        plan.evidence, plan.path
    );
    for i in imports {
        out.push_str(i);
        out.push('\n');
    }
    out.push_str(&format!("\nexport function {}() {{\n", plan.component));
    for h in hooks {
        out.push_str(&format!("  {h}\n"));
    }
    if jsx.trim().is_empty() {
        out.push_str("  return <></>\n}\n");
    } else {
        out.push_str(&format!("  return (\n    <>\n{jsx}    </>\n  )\n}}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest(urls: &[&str]) -> Value {
        json!({ "pages": urls.iter().enumerate().map(|(i, u)| json!({
            "url": u, "final_url": u, "title": "T",
            "source_evidence": [format!("reconstruction/pages/page-{i:05}-375x812.json"), format!("reconstruction/pages/page-{i:05}-1440x900.json")]
        })).collect::<Vec<_>>() })
    }

    #[test]
    fn routes_are_literal_paths_and_the_widest_viewport_is_the_default() {
        let m = manifest(&[
            "https://a.test/",
            "https://a.test/pay",
            "https://a.test/blog/direct",
            "https://a.test/pay?x=1",
        ]);
        let (plans, skipped) = plan(&m, None, None);
        assert_eq!(
            plans.iter().map(|p| p.file.as_str()).collect::<Vec<_>>(),
            ["index", "pay", "blog.direct"]
        );
        assert_eq!(plans[0].component, "HomePage");
        assert_eq!(plans[2].ns, "blog-direct");
        assert!(plans[1].evidence.ends_with("1440x900.json"));
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].reason.contains("same path"));
        let (small, _) = plan(&m, Some("375x812"), None);
        assert!(small[0].evidence.ends_with("375x812.json"));
    }

    #[test]
    fn a_captured_parent_route_does_not_become_the_childs_layout() {
        let m = manifest(&[
            "https://a.test/models/",
            "https://a.test/models/x",
            "https://a.test/models/x/y",
        ]);
        let (plans, _) = plan(&m, None, None);
        let files: Vec<_> = plans.iter().map(|p| p.file.as_str()).collect();
        assert_eq!(files, ["models", "models_.x", "models_.x_.y"]);
        let ids: Vec<_> = plans.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["/models", "/models_/x", "/models_/x_/y"]);
    }

    #[test]
    fn colliding_names_are_skipped_not_overwritten_and_identifiers_cannot_start_with_a_digit() {
        let m = manifest(&[
            "https://a.test/",
            "https://a.test/home",
            "https://a.test/About",
            "https://a.test/about",
            "https://a.test/2024",
        ]);
        let (plans, skipped) = plan(&m, None, None);
        assert_eq!(
            plans.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(),
            ["/", "/About", "/2024"]
        );
        assert_eq!(skipped.len(), 2, "{skipped:?}");
        assert!(skipped.iter().all(|s| s.reason.contains("collides")));
        assert_eq!(plans[2].component, "Route2024Page");
    }

    #[test]
    fn paths_that_are_not_file_routes_are_skipped_with_a_reason() {
        let m = manifest(&[
            "https://a.test/",
            "https://a.test/_next/x",
            "https://a.test/a.b",
            "https://a.test/$id",
            "https://b.test/other",
        ]);
        let (plans, skipped) = plan(&m, None, None);
        assert_eq!(plans.len(), 1);
        assert_eq!(skipped.len(), 4);
        assert!(skipped.iter().any(|s| s.reason.contains("another origin")));
    }

    #[test]
    fn the_route_file_carries_head_document_and_loader() {
        let m = manifest(&["https://a.test/pay"]);
        let (plans, _) = plan(&m, None, None);
        let head = RouteHead {
            title: "Pay — A",
            meta: vec![("name".into(), "description".into(), "x".into())],
            links: vec![vec![
                ("rel".into(), "icon".into()),
                ("href".into(), "/f.ico".into()),
            ]],
            json_ld: vec![r#"{"a":1}"#.into()],
            lang: Some("en"),
            html: vec![("className".into(), "dark".into())],
            body: vec![],
        };
        let src = route_file(&plans[0], &head);
        assert!(src.contains("createFileRoute('/pay')"), "{src}");
        assert!(
            src.contains("{ title: 'Pay — A' }")
                && src.contains("{ name: 'description', content: 'x' }"),
            "{src}"
        );
        assert!(
            src.contains("{ lang: 'en', className: 'dark' }")
                || src.contains("html: { lang: 'en', className: 'dark' }"),
            "{src}"
        );
        assert!(src.contains("type: 'application/ld+json'"), "{src}");
    }
}
