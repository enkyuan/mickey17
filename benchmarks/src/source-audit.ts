import ts from "typescript";
import { readdir, readFile } from "node:fs/promises";
import { join, relative, resolve } from "node:path";

export function inspectSource(file: string, text: string) {
    const ast = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true);
    const imports = new Set<string>(),
        routes = new Set<string>();
    const findings: { kind: string; line: number }[] = [];
    let explicitAny = 0,
        functions = 0,
        jsxElements = 0;
    function visit(node: ts.Node) {
        if (
            (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
            node.moduleSpecifier &&
            ts.isStringLiteral(node.moduleSpecifier)
        )
            imports.add(node.moduleSpecifier.text);
        if (ts.isCallExpression(node)) {
            const name = node.expression.getText(ast),
                first = node.arguments[0];
            if (
                (node.expression.kind === ts.SyntaxKind.ImportKeyword ||
                    name === "require") &&
                first &&
                ts.isStringLiteral(first)
            )
                imports.add(first.text);
            if (
                name === "createFileRoute" &&
                first &&
                ts.isStringLiteral(first)
            )
                routes.add(first.text);
            if (name === "eval" || name === "Function")
                findings.push({
                    kind: "runtime-code-evaluation",
                    line:
                        ast.getLineAndCharacterOfPosition(node.getStart())
                            .line + 1,
                });
        }
        if (
            ts.isNewExpression(node) &&
            node.expression.getText(ast) === "Function"
        )
            findings.push({
                kind: "runtime-code-evaluation",
                line:
                    ast.getLineAndCharacterOfPosition(node.getStart()).line + 1,
            });
        if (
            ts.isJsxAttribute(node) &&
            node.name.getText(ast) === "dangerouslySetInnerHTML"
        )
            findings.push({
                kind: "raw-html-review-required",
                line:
                    ast.getLineAndCharacterOfPosition(node.getStart()).line + 1,
            });
        if (node.kind === ts.SyntaxKind.AnyKeyword) explicitAny++;
        if (
            ts.isFunctionDeclaration(node) ||
            ts.isArrowFunction(node) ||
            ts.isFunctionExpression(node)
        )
            functions++;
        if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node))
            jsxElements++;
        ts.forEachChild(node, visit);
    }
    visit(ast);
    return {
        imports: [...imports],
        routes: [...routes],
        findings,
        explicitAny,
        functions,
        jsxElements,
        lines: text.split("\n").length,
        bytes: Buffer.byteLength(text),
    };
}

/** Structural evidence, never a subjective editability/maintainability score. */
export async function auditSource(project: string, requiredRoutes: string[]) {
    const root = resolve(project),
        pkg = JSON.parse(await readFile(join(root, "package.json"), "utf8"));
    const files: {
        path: string;
        inspection: ReturnType<typeof inspectSource>;
    }[] = [];
    async function walk(dir: string) {
        for (const e of await readdir(dir, { withFileTypes: true })) {
            const path = join(dir, e.name);
            if (e.isSymbolicLink()) throw Error("Symlink in source audit");
            if (e.isDirectory()) await walk(path);
            else if (
                /\.[cm]?[jt]sx?$/.test(e.name) &&
                e.name !== "routeTree.gen.ts"
            )
                files.push({
                    path,
                    inspection: inspectSource(
                        path,
                        await readFile(path, "utf8"),
                    ),
                });
        }
    }
    await walk(join(root, "src"));
    files.sort((a, b) => a.path.localeCompare(b.path));
    const framerImports = files
        .filter((f) =>
            f.inspection.imports.some(
                (i) => i === "framer-motion" || i.startsWith("framer-motion/"),
            ),
        )
        .map((f) => relative(root, f.path));
    const routes = files
        .filter((f) => relative(root, f.path).startsWith("src/routes/"))
        .flatMap((f) => f.inspection.routes)
        .sort();
    // TanStack index-route IDs end in '/', even when trailingSlash: 'never'
    // serves the path without it. This is route identity, not URL normalization:
    // capture still checks the observed browser URL exactly.
    const routeIdentity = (route: string) =>
        route === "/" ? route : route.replace(/\/$/, "");
    const routeCoverage =
        JSON.stringify([...new Set(routes.map(routeIdentity))].sort()) ===
        JSON.stringify([...new Set(requiredRoutes.map(routeIdentity))].sort());
    const recoveredImports = files.flatMap((f) =>
        f.inspection.imports
            .filter((i) => /(?:^|\/)(?:compat|recovered)(?:\/|$)/.test(i))
            .map((module) => ({ file: relative(root, f.path), module })),
    );
    const opaqueFilenames = files
        .filter((f) =>
            /(?:Client|Module|Native)[a-f0-9]{8,}|[a-f0-9]{16,}/i.test(
                relative(root, f.path),
            ),
        )
        .map((f) => relative(root, f.path));
    return {
        files: files.map((f) => relative(root, f.path)),
        sourceFiles: files.map((f) => f.path),
        motionDeclared: Boolean(pkg.dependencies?.motion),
        directFramer: Boolean(
            pkg.dependencies?.["framer-motion"] ||
            pkg.devDependencies?.["framer-motion"] ||
            pkg.optionalDependencies?.["framer-motion"] ||
            pkg.peerDependencies?.["framer-motion"],
        ),
        framerImports,
        routes,
        routeCoverage,
        recoveredImports,
        opaqueFilenames,
        explicitAny: files.reduce((n, f) => n + f.inspection.explicitAny, 0),
        findings: files.flatMap((f) =>
            f.inspection.findings.map((d) => ({
                file: relative(root, f.path),
                ...d,
            })),
        ),
        modules: files.map((f) => ({
            file: relative(root, f.path),
            ...f.inspection,
        })),
        limitation:
            "AST observations and compiler/linter checks do not prove meaningful semantic reconstruction. Fresh CLI and edit/rebuild tests must be reported separately.",
    };
}
