/** Generate the full route/viewport baseline. Gaps must be resolved before approval.
 * These manifests rate existing artifacts, not autonomous generation. Change the
 * generation track and add the real CLI/build commands for an end-to-end run.
 */
import { mkdir, writeFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import type { Suite, Probe } from "../src/schema";
const repo = resolve(Bun.argv[2] ?? "."),
    out = resolve(Bun.argv[3] ?? ".mochi/benchmark-suites");
await mkdir(out, { recursive: true });
const sites = [
    {
        name: "natural",
        origin: "https://www.natural.com",
        port: 4184,
        routes: ["/", "/pay", "/request"],
        capture: ".mochi/trials/20261108-normalized/natural/capture",
    },
    {
        name: "infinitegarden",
        origin: "https://infinitegarden.com",
        port: 4185,
        routes: [
            "/",
            "/articles",
            "/articles/commercial-intelligence-with-infinite-memory",
        ],
        capture: ".mochi/trials/20261108-normalized/infinitegarden/capture",
    },
    {
        name: "microsoft-ai",
        origin: "https://microsoft.ai",
        port: 4186,
        routes: ["/", "/models/", "/models/mai-cyber-1-1-flash/"],
        capture: ".mochi/trials/20261108-microsoft-ai/capture-modules",
    },
];
for (const site of sites) {
    const probes: Probe[] = [];
    const add = (
        id: string,
        category: Probe["category"],
        selector: string,
        property: string,
    ) =>
        probes.push({
            id,
            category,
            reference: selector,
            candidate: selector,
            property,
        });
    for (const prop of ["x", "y", "width", "height"])
        add(`main.${prop}`, "layout", "main", `bounds.${prop}`);
    for (const prop of [
        "font-family",
        "font-size",
        "font-weight",
        "line-height",
        "letter-spacing",
    ])
        add(`heading.${prop}`, "typography", "h1", `style.${prop}`);
    for (const prop of ["color", "background-color"])
        add(`main.${prop}`, "styling", "main", `style.${prop}`);
    add("heading.text", "content", "h1", "text");
    add("page.text", "content", "main", "text");
    add("images.loaded", "assets", "html", "document.images.loaded");
    add("images.count", "assets", "html", "document.images.count");
    add("graphics.svg.count", "graphics", "html", "document.svg.count");
    add("graphics.canvas.count", "graphics", "html", "document.canvas.count");
    add("controls.count", "behavior", "html", "document.controls.count");
    add("page.path", "routing", "html", "document.path");
    add("page.overflow", "layout", "html", "document.overflow");
    const suite: Suite = {
        schemaVersion: 1,
        name: site.name,
        referenceOrigin: site.origin,
        candidateOrigin: `http://localhost:${site.port}`,
        referenceKind: "live",
        candidateProject: join(repo, ".mochi/projects", site.name),
        candidateBuild: join(repo, ".mochi/projects", site.name, ".output"),
        generationMode: "existing-project",
        sourceCapture: join(repo, site.capture),
        requiredRoutes: site.routes,
        repeats: 3,
        coverageGaps: [
            "Live reference is not the immutable captured production build.",
            "Inventory probe counts do not qualify individual controls or graphics. Add all source-observed state traces.",
            "Continuous SVG/WebGL animation clocks and GPU output need synchronized source-state capture.",
            "No edit/rebuild verification or reconstruction command in this existing-project track.",
        ],
        cases: [],
        reactDoctor: {
            command: ["bunx", "--bun", "react-doctor@0.9.22"],
            cwd: join(repo, ".mochi/projects", site.name),
        },
    };
    for (const [index, route] of site.routes.entries())
        for (const viewport of [
            [390, 844],
            [768, 1024],
            [1440, 900],
            [1960, 1190],
        ] as [number, number][]) {
            // Reference DOM observations: the Garden essay uses an article root;
            // Microsoft's models title is h2, with an explicit unsplit text node.
            const routeProbes = probes.map((probe) => {
                if (
                    site.name === "infinitegarden" &&
                    route.startsWith("/articles/commercial-") &&
                    probe.reference === "main"
                )
                    return {
                        ...probe,
                        reference: "article",
                        candidate: "article",
                    };
                if (
                    site.name === "microsoft-ai" &&
                    route === "/models/" &&
                    probe.reference === "h1"
                )
                    return {
                        ...probe,
                        reference:
                            probe.property === "text"
                                ? "main h2 [data-split-original]"
                                : "main h2",
                    };
                return probe;
            });
            for (const [phase, y] of [
                ["initial", 0],
                ["one-screen", viewport[1]],
                ["two-screens", viewport[1] * 2],
                ["four-screens", viewport[1] * 4],
                ["eight-screens", viewport[1] * 8],
                ["bottom", 1e9],
            ] as const)
                suite.cases.push({
                    id: `route-${index}.${viewport[0]}.${phase}`,
                    route,
                    viewport,
                    reducedMotion: false,
                    actions: y ? [{ kind: "scroll", y }] : [],
                    probes: routeProbes,
                });
            suite.cases.push({
                id: `route-${index}.${viewport[0]}.reduced`,
                route,
                viewport,
                reducedMotion: true,
                actions: [],
                probes: routeProbes,
            });
        }
    await writeFile(
        join(out, site.name + ".json"),
        JSON.stringify(suite, null, 2),
    );
}
