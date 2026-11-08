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

pub(super) fn editing_guide(report: &Report) -> Result<String> {
    let mut guide = String::from(
        "# Editing this reconstruction\n\nThis is a compatibility reconstruction, not recovered authored application source. Formatting and file-based routes do not qualify it as a maintainable semantic reconstruction.\n\n## Where changes belong\n\n- `src/routes/`: TanStack Start physical file routes. Add a route here and rebuild to regenerate the route tree.\n- `src/features/pages/`: route content and document metadata.\n- `src/components/`: extracted structural candidates and recovered client entry components; names and props may still be mechanical.\n- `src/components/ui/button.tsx`: Base UI Button with normal React props. Other recovered controls are not converted to Base UI.\n- `src/compat/recovered/`: formatted deployed programs. Short names and bundled dependencies remain; this is application runtime source, so it must be committed.\n- `src/styles/compat/`: captured cascades, intentionally global; not component-scoped CSS Modules.\n- `src/styles/tokens.css`: owner-authored tokens and scoped overrides, loaded after the selected document cascade. Empty by default because semantic tokens have not been inferred.\n- `public/`: required images and fonts; commit them.\n- `.mochi/`: ignored build/evidence metadata, including `export.json` and `style-audit.json`. These are not runtime dependencies.\n\n## Captured routes\n\n| URL | Route file | Feature directory |\n| --- | --- | --- |\n",
    );
    for path in &report.captured_paths {
        let path_only = path.split('?').next().unwrap_or(path).trim_end_matches('/');
        guide.push_str(&format!(
            "| {} | `src/routes/{}` | `src/features/pages{}` |\n",
            js(path)?,
            route_file(path_only),
            path_only
        ));
    }
    guide.push_str("\n## Prove an edit\n\nUse `bun install --frozen-lockfile`, `bun run build`, `bun run check`, then `bun start`. Change visible text in the relevant feature/component and verify both the server HTML and hydrated page. Edit a known CSS custom property in `tokens.css` and verify its computed value on desktop and mobile. Add a route file and verify a direct request and client navigation. Restore probe edits after testing.\n\n## Still required for authored-source quality\n\nReplace recovered controls with Base UI compositions and explicit state. Extract meaningful typed props and data. Rebuild fonts and semantic tokens with scoped responsive/theme definitions. Retain custom graphics in named feature modules/hooks. Verify keyboard/focus behavior, scroll behavior, responsive states, and screenshots after each replacement. Do not declare a conversion complete while the route still depends on recovered bundled programs.\n");
    Ok(guide)
}

#[cfg(test)]
mod tests {
    use super::*;
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
