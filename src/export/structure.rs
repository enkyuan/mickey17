//! Source ownership boundaries. These do not imply semantic reconstruction.
use super::{Report, js};
use crate::{evidence::Content, project::Project};
use anyhow::{Result, ensure};
use std::{collections::BTreeMap, path::Path};

pub(crate) fn route_file(path: &str) -> String {
    let segments = path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| {
            if matches!(s, "index" | "route") {
                format!("[{s}]")
            } else {
                // Dots are separators unless individually escaped. Wrapping
                // an entire dotted segment does not protect its dots.
                let value = s.replace('.', "[.]").replace('_', "[_]");
                if let Some(rest) = value.strip_prefix('-') {
                    format!("[-]{rest}")
                } else {
                    value
                }
            }
        })
        .collect::<Vec<_>>();
    if segments.is_empty() {
        "index.tsx".into()
    } else {
        format!("{}/index.tsx", segments.join("/"))
    }
}

pub(super) fn page_name(path: &str) -> String {
    let mut name = String::new();
    for word in path
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let mut chars = word.chars();
        name.extend(chars.next().unwrap().to_uppercase());
        name.extend(chars);
    }
    if name.is_empty() {
        name = "Home".into();
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert_str(0, "Route");
    }
    name + "Page"
}

pub(crate) fn format_source(path: &str, source: &str) -> Result<String> {
    if !matches!(
        Path::new(path).extension().and_then(|e| e.to_str()),
        Some("ts" | "tsx")
    ) {
        return Ok(source.into());
    }
    let arena = oxc_allocator::Allocator::default();
    let kind = oxc_span::SourceType::from_path(path)?;
    let parsed = oxc_parser::Parser::new(&arena, source, kind).parse();
    ensure!(
        parsed.diagnostics.is_empty(),
        "invalid generated source: {path}"
    );
    let formatted = oxc_codegen::Codegen::new()
        .with_options(oxc_codegen::CodegenOptions {
            single_quote: true,
            indent_char: oxc_codegen::IndentChar::Space,
            indent_width: 2,
            ..Default::default()
        })
        .build(&parsed.program)
        .code;
    ensure!(
        formatted.len() <= 16 * 1024 * 1024,
        "formatted source exceeds file budget"
    );
    Ok(formatted)
}

pub(super) fn style_audit(model: &Project) -> Result<String> {
    // Inventory observed values with context; equal pixels do not establish
    // common semantic tokens. Never merge viewport/theme states here.
    let mut values: BTreeMap<String, BTreeMap<String, Vec<serde_json::Value>>> = BTreeMap::new();
    for observation in model.observations.values() {
        for (index, node) in observation.capture.nodes.iter().enumerate() {
            let Content::Element { style, .. } = &node.content else {
                continue;
            };
            for (property, value) in &observation.capture.styles[*style] {
                if !(property.starts_with("--")
                    || matches!(
                        property.as_str(),
                        "color"
                            | "background-color"
                            | "font-family"
                            | "font-size"
                            | "font-weight"
                            | "line-height"
                            | "border-radius"
                            | "gap"
                    ))
                {
                    continue;
                }
                let examples = values
                    .entry(property.clone())
                    .or_default()
                    .entry(value.clone())
                    .or_default();
                if examples.len() < 4 {
                    examples.push(serde_json::json!({"url":observation.capture.url,"viewport":observation.capture.viewport,"scheme":observation.capture.color_scheme,"node":index}));
                }
            }
        }
    }
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status":"observed_values_not_semantic_tokens",
        "values":values,
        "policy":"Retain selector, cascade, viewport, theme and font context. Do not globally merge equal values or rename custom properties without verifying every consumer."
    }))?)
}

/// Facts about the capture that bound what the export can claim.
pub(super) struct Conditions {
    pub color_schemes: Vec<String>,
    pub states: Vec<String>,
    pub viewports: Vec<String>,
    pub recovered_routes: Vec<String>,
}
impl Conditions {
    pub(super) fn observe(model: &Project, recovery: Option<&crate::recover::Recovery>) -> Self {
        let mut schemes = std::collections::BTreeSet::new();
        let mut states = std::collections::BTreeSet::new();
        let mut viewports = std::collections::BTreeSet::new();
        for observation in model.observations.values() {
            let capture = &observation.capture;
            schemes.insert(capture.color_scheme.clone());
            states.insert(capture.state.clone());
            viewports.insert((capture.viewport.width, capture.viewport.height));
        }
        Self {
            color_schemes: schemes.into_iter().collect(),
            states: states.into_iter().collect(),
            viewports: viewports
                .into_iter()
                .map(|(w, h)| format!("{w}x{h}"))
                .collect(),
            recovered_routes: recovery
                .map(|r| r.routes.keys().cloned().collect())
                .unwrap_or_default(),
        }
    }
    fn recovered(&self) -> bool {
        !self.recovered_routes.is_empty()
    }
}

pub(super) const LINK_POLICY: &str = "Links to captured routes become local paths. A link to a page that was not captured keeps its absolute live-origin URL (it leaves the export and works only while the original site does); it is never rewritten to a local path that would 404. Images and other resources that were not captured stay at their remote URL. Every such URL is listed in external_references with its kind and occurrence count.";

/// Plain statements of what this export is and is not. Reused by the report,
/// README and EDITING.md so the three cannot drift apart.
pub(super) fn limitations(c: &Conditions) -> Vec<String> {
    let mut out = Vec::new();
    if c.recovered() {
        out.push(format!(
            "Routes {} run recovered, formatted copies of the deployed client programs (src/compat/recovered/); other routes are static replicas. Recovered code keeps its short names and bundled dependencies and is not authored source.",
            c.recovered_routes.join(", ")
        ));
    } else {
        out.push("This is a static structural replica: the DOM and cascade observed at capture time, serialized as React elements. No application scripts, controllers, recovered runtime, effects, timers, scroll or pointer logic, or animation drivers exist in the project (src/compat/recovered/ is not generated); the only client code is the shared runtime: src/lib/runtime.tsx (link navigation, viewport-variant switching, canvas snapshot painting, shadow-root attachment), src/lib/document.ts (per-viewport <html>/<body> attributes) and the form-submit guard in src/routes/__root.tsx. The `motion` dependency is installed but not imported.".into());
    }
    // Recovered routes run the deployed programs, so static-replica statements
    // must not be made about them.
    let scope = if c.recovered() {
        "On static (non-recovered) routes: "
    } else {
        ""
    };
    out.push(format!("{scope}Animation is not replayed. Where the capture recorded animation state, its observed property values were written into inline `style` strings (diagnostic observed_animation_state_preserved), so a route shows its captured end state and nothing continues or reacts to input."));
    out.push(format!("{scope}Canvas, WebGL and video surfaces are static PNG snapshots (data-mochi-surface=\"static-snapshot\"); frames are sandboxed srcDoc documents; shadow roots are re-attached after hydration; runtime-only elements with no visual output are omitted. The `surfaces` list records each one per route and viewport."));
    out.push(format!("{scope}Buttons have no click handlers and `action`, `formaction` and `on*` attributes are removed. No backend is recreated, and the root route cancels every form submit (with one console message) so a submit does not reload the page with field values in the URL."));
    if c.recovered() {
        out.push("Recovered routes keep the deployed programs' own animation, drawing and event code; their capture surfaces are not replaced by snapshots. Form submits are still cancelled by the root route because no backend exists.".into());
    }
    out.push(format!(
        "Observed only with color scheme(s) {} in state(s) {} at viewport(s) {}. Other color schemes, hover/focus/open/scrolled states, authenticated states and other viewports were not captured and are not reproduced.",
        c.color_schemes.join(", "),
        c.states.join(", "),
        c.viewports.join(", ")
    ));
    out.push("Viewport variants: each captured viewport is a separate full DOM. Server rendering picks the smallest capture for mobile requests (Sec-CH-UA-Mobile, else User-Agent) and the largest otherwise, and every page response carries `Vary: Sec-CH-UA-Mobile, User-Agent`. Desktop windows up to about 907px wide (for 375/1440 captures) and desktop-UA tablets such as iPadOS receive the desktop DOM from the server and remount to the nearest capture after hydration; <html>/<body> attributes follow the same selection through the shared policy.ts variantIndex. Hydration and viewport switching have NOT been verified in a browser.".into());
    out.push("Routes are literal captured pathname+query states; the runtime falls back to the same path with the opposite trailing slash and then to the path without a query. Percent-encoded paths are decoded once, unknown query strings and trailing-slash variants serve the captured page; paths that were never captured return not found. No dynamic parameters or nesting are inferred.".into());
    out.push("Document head: title, description, Open Graph, Twitter, canonical, icon links and JSON-LD are emitted from the capture. Because the DOM snapshot keeps only <title> of <head>, icons and JSON-LD are read from the mirrored HTML response saved with the capture. Framework state (csp-nonce, next-size-adjust) and runtime elements (next-route-announcer) are dropped. canonical, og:url and JSON-LD still name the original site's origin. The mirrored HTML and icon files behind the head are not covered by `input_sha256`; their SHA-256 digests are recorded under `head_inputs` in the report, and export refuses only if they change while it runs. JSON-LD keeps the author's key order.".into());
    out.push("Self-containment: uncaptured same-origin links keep absolute live-origin URLs and uncaptured remote images stay remote (see link_policy and external_references). The capture holds no bytes for remote hosts, so they cannot be localized. `external_references` lists each distinct URL once with every kind that reaches it (including absolute URLs in emitted meta and canonical); `capture_time_requests` separately lists third-party and runtime requests seen while capturing, which the generated source does not use. SVG files that contain scripts, event handlers, foreignObject, XHTML, script-targeting animation or external references, or that could hide them (namespace prefixes, character references, DTD internal subsets, xml-stylesheet/XSLT, CSS escapes, non-UTF-8 bytes), are not copied to public/ (diagnostic asset_svg_not_copied); references to them stay at their original URL.".into());
    out.push("Inline-style `url()` accepts raster data images only, and CSS-escape or `image-set` bypasses of that rule are a known unfixed limitation. Compatibility CSS preserves global selectors and the original cascade per document. Semantic tokens, meaningful prop names and component-local selector extraction are not reconstructed; component props remain positional (prop_N).".into());
    out.push("CSS imports, escaped/nested CSS URLs, multi-select and unsupported route/tag syntax are refused. Only passive local image/font candidates are copied; query-bearing mirror assets stay unresolved.".into());
    out
}

fn behavior_section(report: &Report) -> String {
    if report.client_recovery.is_some() {
        "## What this is\n\nA compatibility reconstruction with **explicitly recovered client programs** on some routes (see `limitations` in `.mochi/export.json`). Routes with `src/compat/recovered/` run formatted copies of the deployed bundles; every other route is a static replica. Neither is authored application source.\n".into()
    } else {
        "## What this is\n\nA **static structural replica** of what was captured, not a working application:\n\n- The DOM and cascade of each captured route and viewport are serialized as React elements. Animation appears only as the end-state values captured into inline `style` strings.\n- There are **no controllers or recovered runtime**: `src/compat/recovered/` does not exist, no captured or recovered effect, timer, scroll or pointer logic exists, nothing from `motion` is imported (it is installed only), and buttons have no click handlers. The only client code is the shared runtime: `src/lib/runtime.tsx` (in-app link navigation, switching between captured viewport variants, painting canvas snapshots, attaching shadow roots), `src/lib/document.ts` (per-viewport `<html>`/`<body>` attributes) and the form-submit guard in `src/routes/__root.tsx`.\n- Canvas/WebGL surfaces are **static PNG snapshots** (`data-mochi-surface=\"static-snapshot\"`); `.mochi/export.json` `surfaces` lists which ones, per route and viewport.\n- Forms do not submit. `src/routes/__root.tsx` cancels every submit because `action`/`on*` attributes were removed and no backend was captured; it logs one console message.\n- Captured with the color scheme(s), state(s) and viewport(s) listed in `limitations`; nothing else (hover, focus, scroll positions, light/dark switching, other widths) was observed.\n- SSR is viewport-aware only by request hint (mobile UA/Client Hint -> smallest capture, else largest); `<html>`/`<body>` attributes follow it. Uncaptured same-origin links keep absolute live-origin URLs and uncaptured remote images stay remote; `external_references` lists them all.\n".into()
    }
}

pub(super) fn readme(report: &Report) -> Result<String> {
    Ok(format!(
        "# Reconstructed frontend\n\nTanStack Start with physical file routes and Bun. Read [EDITING.md](EDITING.md) for the route map, edit locations, and remaining reconstruction debt.\n\n```sh\nbun install --frozen-lockfile\nbun run build\nbun run check\nbun start\n```\n\nFor development: `bun run dev`. `bun run dev` serves at http://127.0.0.1:3000 (strict port; set `PORT` to change it). `bun start` runs the built Nitro server on port 3000 (`PORT` or `NITRO_PORT` to change it) and binds Nitro's default host, which is not limited to loopback. Run one project at a time. `bun run check` generates the route tree (`src/routeTree.gen.ts`, committed) before type-checking, so install, check, build works on a fresh checkout; `bun run clean` removes only regenerable output (`.output`, `.tanstack`, `.mochi/dist`, `node_modules/.vite`, `.nitro`, `.vite-temp`). Bun 1.4.4 is the pinned toolchain.\n\n{}\nRoutes delegate to feature pages in `src/features/pages`; each `data.ts` holds the document title, meta, canonical/icon links and JSON-LD per route. Captured CSS stays in `src/styles/compat` as compatibility source, not an authored Base UI application or design system.\n\nThe export command does not install dependencies or verify runtime behavior. `.mochi/export.json` lists provenance, limitations, per-surface replacements and every external URL; `.mochi/style-audit.json` lists observed style values. These reports and build output are ignored; application source and required public assets are not.\n",
        behavior_section(report)
    ))
}

pub(super) fn editing_guide(report: &Report) -> Result<String> {
    let recovered = report.client_recovery.is_some();
    let mut guide = format!(
        "# Editing this reconstruction\n\nThis is a compatibility reconstruction, not recovered authored application source. Formatting and file-based routes do not qualify it as a maintainable semantic reconstruction.\n\n{}\n## Where changes belong\n\n- `src/routes/`: TanStack Start physical file routes. Add a route here and rebuild to regenerate the route tree. `__root.tsx` renders `<html>`/`<body>` and cancels form submits.\n- `src/features/pages/`: route content (`page.tsx`) and per-route document data (`data.ts`: title, meta, canonical and icon links, JSON-LD, `<html>`/`<body>` attributes per captured viewport).\n- `src/lib/document.ts`: how document data becomes head tags and which viewport variant of `<html>`/`<body>` attributes applies.\n- `src/components/`: extracted structural candidates, named from `data-component` when the page has it and otherwise `ComponentN`; props are positional (`prop_N`).\n- `src/components/ui/button.tsx`: Base UI Button with normal React props. Nothing else is converted to Base UI.\n",
        behavior_section(report)
    );
    if recovered {
        guide.push_str("- `src/compat/recovered/`: formatted deployed programs for the recovered routes. Short names and bundled dependencies remain; this is application runtime source, so it must be committed. `src/components/Client*.tsx` are the entry components that mount them.\n");
    } else {
        guide.push_str("- There is no `src/compat/recovered/`: no client program was recovered for this export.\n");
    }
    guide.push_str("- `src/styles/compat/`: captured cascades, intentionally global; not component-scoped CSS Modules.\n- `src/styles/tokens.css`: owner-authored tokens and scoped overrides, loaded after the selected document cascade. Empty by default because semantic tokens have not been inferred.\n- `public/`: required images, icons and fonts; commit them.\n- `.mochi/`: ignored build/evidence metadata, including `export.json` (limitations, surfaces, external references, diagnostics) and `style-audit.json`. These are not runtime dependencies.\n\n## Captured routes\n\n| URL | Route file | Feature directory |\n| --- | --- | --- |\n");
    for path in &report.captured_paths {
        let path_only = path.split('?').next().unwrap_or(path).trim_end_matches('/');
        guide.push_str(&format!(
            "| {} | `src/routes/{}` | `src/features/pages{}` |\n",
            js(path)?,
            route_file(path_only),
            path_only
        ));
    }
    guide.push_str("\n## Known limitations\n\n");
    for limitation in &report.limitations {
        guide.push_str(&format!("- {limitation}\n"));
    }
    // Only raster surfaces become snapshots; the others are omitted or re-hosted.
    let kind = |k: &str| report.surfaces.iter().filter(|s| s.kind == k).count();
    guide.push_str(&format!("\n## Replaced surfaces\n\n{} surface record(s) in `.mochi/export.json` under `surfaces`: {} static snapshot(s), {} omitted runtime-only element(s), {} sandboxed frame(s), {} shadow root(s). {} distinct URL(s) outside the export under `external_references` and {} capture-time request(s) under `capture_time_requests`.\n", report.surfaces.len(), kind("raster"), kind("nonvisual"), kind("frame"), kind("shadow"), report.external_references.len(), report.capture_time_requests.len()));
    guide.push_str("\n## Prove an edit\n\nUse `bun install --frozen-lockfile`, `bun run build`, `bun run check`, then `bun start`. Change visible text in the relevant feature/component and verify both the server HTML and hydrated page. Edit a known CSS custom property in `tokens.css` and verify its computed value on desktop and mobile. Add a route file and verify a direct request and client navigation. Restore probe edits after testing.\n\n## Still required for authored-source quality\n\nImplement the behavior the capture could not preserve: interactions, animation drivers, canvas/WebGL drawing and form handling. Replace the static snapshots with owned drawing code, replace compatibility controls with Base UI compositions and explicit state, extract meaningful typed props and data, rebuild fonts and semantic tokens with scoped responsive/theme definitions, and verify keyboard/focus behavior, scroll behavior, responsive states, and screenshots after each replacement.");
    if recovered {
        guide.push_str(" Do not declare a conversion complete while a route still depends on recovered bundled programs.");
    }
    guide.push('\n');
    Ok(guide)
}

/// Remove unused locals from generated TypeScript: `const base` declarations
/// that nothing reads and import bindings that the file body never mentions.
/// Purely textual and conservative: a name counts as used if it appears anywhere
/// else as a whole word.
pub(super) fn prune_unused(source: &str) -> String {
    static WORD: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"[A-Za-z_$][A-Za-z0-9_$]*").expect("word"));
    // Generated text and attribute values are JSON strings; their words are not code.
    static STRING: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#""(?:[^"\\]|\\.)*""#).expect("string literal")
    });
    let words = |text: &str| -> std::collections::BTreeMap<String, usize> {
        let mut counts = std::collections::BTreeMap::new();
        let text = STRING.replace_all(text, "\"\"");
        for m in WORD.find_iter(&text) {
            *counts.entry(m.as_str().to_owned()).or_insert(0) += 1;
        }
        counts
    };
    let mut source = source.to_owned();
    for (declaration, mentions) in [("const base=useSource();", 1), ("const base=page.base;", 2)] {
        if source.matches(declaration).count() == 1 && words(&source).get("base") == Some(&mentions)
        {
            source = source.replace(declaration, "");
        }
    }
    let (imports, body): (Vec<&str>, Vec<&str>) = source
        .lines()
        .partition(|line| line.starts_with("import ") && line.contains(" from "));
    let used = words(&body.join("\n"));
    let mut out = String::new();
    for line in imports {
        let Some((clause, from)) = line[7..].rsplit_once(" from ") else {
            out.push_str(line);
            out.push('\n');
            continue;
        };
        let (type_only, clause) = clause
            .strip_prefix("type ")
            .map_or((false, clause), |rest| (true, rest));
        let clause = clause.trim();
        let identifier = |name: &str| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
        };
        let binding = |item: &str| {
            item.rsplit(" as ")
                .next()
                .unwrap_or(item)
                .trim_start_matches("type ")
                .trim()
                .to_owned()
        };
        let named = clause
            .strip_prefix('{')
            .and_then(|c| c.strip_suffix('}'))
            .map(|named| {
                named
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .collect::<Vec<_>>()
            })
            // Anything that is not a plain list of bindings is kept as written.
            .filter(|items| {
                !items.is_empty() && items.iter().all(|item| identifier(&binding(item)))
            });
        if let Some(items) = named {
            let kept: Vec<&str> = items
                .into_iter()
                .filter(|item| used.contains_key(&binding(item)))
                .collect();
            if !kept.is_empty() {
                out.push_str(&format!(
                    "import {}{{{}}} from {from}\n",
                    if type_only { "type " } else { "" },
                    kept.join(",")
                ));
            }
        } else if used.contains_key(clause)
            // Only a lone default binding is judged; `a, {b}`, `* as ns` and
            // anything else this does not parse are kept as written.
            || !identifier(clause)
        {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&body.join("\n"));
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unused_imports_and_locals_are_pruned_but_text_never_keeps_them_alive() {
        let source = "import {Anchor,elementProps,type Control,ShadowHost} from '@/lib/runtime'\nimport {Button} from '@/components/ui/button'\nimport css0 from '@/styles/a.css?inline'\nexport function A(props:{x:string}){const base=useSource();return (<p>{\"Control ShadowHost base\"}<Button {...elementProps('button',{},null,base)}/></p>)}\n";
        let pruned = prune_unused(source);
        assert!(
            pruned.starts_with("import {elementProps} from '@/lib/runtime'\nimport {Button}"),
            "{pruned}"
        );
        assert!(!pruned.contains("css0") && pruned.contains("const base=useSource();"));
        let unused = "import {useSource} from '@/lib/runtime'\nexport function B(){const base=useSource();return (<p>{\"base\"}</p>)}\n";
        let pruned = prune_unused(unused);
        assert!(
            !pruned.contains("const base") && !pruned.contains("import"),
            "{pruned}"
        );
        let page = "export function P({page}){const base=page.base;switch(page.key){case 'a':return <S value={page.base}/>}return null}\n";
        assert!(prune_unused(page).contains("const base=page.base;"));
    }
    #[test]
    fn imports_the_pruner_cannot_parse_are_kept_as_written() {
        let source = "import React, {useState} from 'react'\nimport {a} from 'x'; import {b} from 'y'\nimport {} from './effects'\nimport * as ns from 'z'\nexport function C(){const [s]=useState(a+b);return <p>{s}</p>}\n";
        let pruned = prune_unused(source);
        for line in source.lines().take(4) {
            assert!(pruned.contains(line), "{line} dropped:\n{pruned}");
        }
    }
    #[test]
    fn physical_routes_preserve_literals_without_inventing_layouts() {
        assert_eq!(route_file("/"), "index.tsx");
        assert_eq!(
            route_file("/models/mai-cyber/"),
            "models/mai-cyber/index.tsx"
        );
        assert_eq!(
            route_file("/_private/index/a.b/route"),
            "[_]private/[index]/a[.]b/[route]/index.tsx"
        );
        assert_eq!(page_name("/models/mai-cyber"), "ModelsMaiCyberPage");
        assert_eq!(page_name("/2026"), "Route2026Page");
        assert_eq!(route_file("/-draft"), "[-]draft/index.tsx");
    }
}
