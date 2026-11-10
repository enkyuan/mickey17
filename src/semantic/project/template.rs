//! Files of the assembled project that do not depend on the capture. The package manifest, lock
//! file, build configuration and route-tree script are the compat exporter's own template files,
//! read-only, so `bun install --frozen-lockfile` stays valid and both generators share one stack.
pub const PACKAGE_JSON: &str = include_str!("../../export_template/package.json");
pub const BUN_LOCK: &str = include_str!("../../export_template/bun.lock");
pub const TSCONFIG: &str = include_str!("../../export_template/tsconfig.json");
pub const VITE_CONFIG: &str = include_str!("../../export_template/vite.config.ts");
pub const BUN_VERSION: &str = include_str!("../../export_template/.bun-version");
pub const ROUTE_TREE_SCRIPT: &str =
    include_str!("../../export_template/scripts/generate-route-tree.ts");

/// Build output stays out of version control; `.mochi/provenance` and `.mochi/semantic` are evidence.
pub const GITIGNORE: &str = "node_modules/\n.output/\ndist/\n.tanstack/\n.mochi/dist/\n";

pub const ROUTER: &str = "import { createRouter } from '@tanstack/react-router'\nimport { routeTree } from './routeTree.gen'\n\nexport function getRouter() {\n  return createRouter({ routeTree, scrollRestoration: true, caseSensitive: true, trailingSlash: 'preserve' })\n}\n\ndeclare module '@tanstack/react-router' {\n  interface Register {\n    router: ReturnType<typeof getRouter>\n  }\n}\n";

pub const DOCUMENT_TS: &str = "/** `<html>` and `<body>` attributes of a captured route, as JSX property names. */\nexport type RouteDocument = {\n  html: Record<string, string>\n  body: Record<string, string>\n}\n";

pub const APP_CSS: &str = "/* Tailwind v4 theme layer and utilities. Nothing in the assembled markup uses a Tailwind\n * utility yet; the project report says where the page styles come from. */\n@layer theme, base, components, utilities;\n@import \"tailwindcss/theme.css\" layer(theme);\n@import \"tailwindcss/utilities.css\" layer(utilities);\n";

/// The root route: one document for every captured route. `imports` are the style imports.
pub fn root_route(imports: &[String]) -> String {
    let mut out = String::from("import '@/styles/app.css'\n");
    for i in imports {
        out.push_str(&format!("import '{i}'\n"));
    }
    out.push_str(
        "import { createRootRoute, HeadContent, Outlet, Scripts, useRouterState } from '@tanstack/react-router'\nimport type { ComponentProps } from 'react'\nimport type { RouteDocument } from '@/lib/document'\n\nexport const Route = createRootRoute({\n  head: () => ({ meta: [{ charSet: 'utf-8' }] }),\n  component: Document,\n  notFoundComponent: () => (\n    <main>\n      <h1>Page not captured</h1>\n    </main>\n  ),\n})\n\nfunction Document() {\n  const captured = useRouterState({\n    select: (state) =>\n      [...state.matches]\n        .reverse()\n        .map((match) => (match.loaderData as { document?: RouteDocument } | undefined)?.document)\n        .find(Boolean),\n  })\n  return (\n    <html {...(captured?.html as ComponentProps<'html'> | undefined)}>\n      <head>\n        <HeadContent />\n      </head>\n      <body {...(captured?.body as ComponentProps<'body'> | undefined)}>\n        <Outlet />\n        <Scripts />\n      </body>\n    </html>\n  )\n}\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stack_is_the_exporters_own_and_the_root_route_is_complete() {
        // Same manifest and lock as the compat exporter, so the two generators share one stack.
        assert!(PACKAGE_JSON.contains("\"@base-ui/react\": \"1.9.0\""));
        assert!(PACKAGE_JSON.contains(
            "\"check\": \"bun --bun scripts/generate-route-tree.ts && bun --bun tsc --noEmit\""
        ));
        assert!(VITE_CONFIG.contains("port: Number(process.env.PORT) || 3000"));
        let root = root_route(&["@/styles/rewritten/styles.css".into()]);
        assert!(
            root.starts_with("import '@/styles/app.css'\nimport '@/styles/rewritten/styles.css'\n")
        );
        for needle in [
            "HeadContent",
            "Scripts",
            "Outlet",
            "notFoundComponent",
            "charSet: 'utf-8'",
        ] {
            assert!(root.contains(needle), "{needle}");
        }
        assert!(
            GITIGNORE.contains(".mochi/dist/") && !GITIGNORE.contains("\n.mochi/\n"),
            "provenance stays tracked"
        );
    }
}
