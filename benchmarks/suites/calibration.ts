import { writeFile } from "node:fs/promises";
import type { Suite, Probe } from "../src/schema";
const probes: Probe[] = [
    {
        id: "hero.x",
        category: "layout",
        reference: "#hero",
        candidate: "#hero",
        property: "bounds.x",
    },
    {
        id: "heading.x",
        category: "layout",
        reference: "h1",
        candidate: "h1",
        property: "bounds.x",
    },
    {
        id: "cards.second.x",
        category: "layout",
        reference: "#card-second",
        candidate: "#card-second",
        property: "bounds.x",
    },
    {
        id: "heading.tracking",
        category: "typography",
        reference: "h1",
        candidate: "h1",
        property: "style.letter-spacing",
    },
    {
        id: "heading.font",
        category: "typography",
        reference: "h1",
        candidate: "h1",
        property: "style.font-size",
    },
    {
        id: "body.color",
        category: "styling",
        reference: "body",
        candidate: "body",
        property: "style.color",
    },
    {
        id: "copy",
        category: "content",
        reference: "#copy",
        candidate: "#copy",
        property: "text",
    },
    {
        id: "image.loaded",
        category: "assets",
        reference: "#asset",
        candidate: "#asset",
        property: "image.loaded",
    },
    {
        id: "path.geometry",
        category: "graphics",
        reference: "#connection",
        candidate: "#connection",
        property: "path.d",
    },
    {
        id: "path.progress",
        category: "graphics",
        reference: "#connection",
        candidate: "#connection",
        property: "style.stroke-dashoffset",
    },
    {
        id: "menu.expanded",
        category: "behavior",
        reference: "#menu-trigger",
        candidate: "#menu-trigger",
        property: "attribute.aria-expanded",
    },
    {
        id: "details.href",
        category: "routing",
        reference: "#details",
        candidate: "#details",
        property: "attribute.href",
    },
];
const suite: Suite = {
    schemaVersion: 1,
    name: "calibration-identical-reference",
    referenceOrigin: "http://127.0.0.1:4197",
    candidateOrigin: "http://127.0.0.1:4198",
    referenceKind: "fixture",
    generationMode: "fixture",
    requiredRoutes: ["/", "/details"],
    coverageGaps: [],
    repeats: 3,
    cases: [],
};
for (const viewport of [
    [1440, 900],
    [390, 844],
] as [number, number][]) {
    for (const state of ["initial", "scroll", "menu", "reduced"])
        suite.cases.push({
            id: `home.${viewport[0]}.${state}`,
            route: "/",
            viewport,
            reducedMotion: state === "reduced",
            actions:
                state === "scroll"
                    ? [{ kind: "scroll", y: 400 }]
                    : state === "menu"
                      ? [
                            {
                                kind: "click",
                                reference: "#menu-trigger",
                                candidate: "#menu-trigger",
                            },
                        ]
                      : [],
            probes,
        });
}

for (const viewport of [
    [1440, 900],
    [390, 844],
] as [number, number][]) {
    for (const reducedMotion of [false, true])
        suite.cases.push({
            id: `details.${viewport[0]}.${reducedMotion}`,
            route: "/details",
            viewport,
            reducedMotion,
            actions: [],
            probes,
        });
    const click = (selector: string) => ({
        kind: "click" as const,
        reference: selector,
        candidate: selector,
    });
    const behaviorProbes: Probe[] = [
        ...probes,
        {
            id: "dialog.open",
            category: "behavior",
            reference: "#dialog",
            candidate: "#dialog",
            property: "has-attribute.open",
        },
        {
            id: "email.valid",
            category: "behavior",
            reference: "input[name=email]",
            candidate: "input[name=email]",
            property: "validity.valid",
        },
        {
            id: "page.path",
            category: "routing",
            reference: "html",
            candidate: "html",
            property: "document.path",
        },
    ];
    suite.cases.push({
        id: `home.${viewport[0]}.menu-escape`,
        route: "/",
        viewport,
        reducedMotion: false,
        actions: [click("#menu-trigger"), { kind: "press", key: "Escape" }],
        probes: [
            ...behaviorProbes,
            {
                id: "menu.focus",
                category: "behavior",
                reference: "#menu-trigger",
                candidate: "#menu-trigger",
                property: "focused",
            },
        ],
    });
    suite.cases.push({
        id: `home.${viewport[0]}.dialog-invalid`,
        route: "/",
        viewport,
        reducedMotion: false,
        actions: [
            click("#menu-trigger"),
            click("#open-dialog"),
            {
                kind: "fill",
                reference: "input[name=email]",
                candidate: "input[name=email]",
                value: "invalid-email",
            },
        ],
        probes: behaviorProbes,
    });
    suite.cases.push({
        id: `home.${viewport[0]}.navigate`,
        route: "/",
        expectedRoute: "/details",
        viewport,
        reducedMotion: false,
        actions: [click("#menu-trigger"), click("#details")],
        probes: behaviorProbes,
    });
    suite.cases.push({
        id: `home.${viewport[0]}.reverse-scroll`,
        route: "/",
        viewport,
        reducedMotion: false,
        actions: [
            { kind: "scroll", y: 800 },
            { kind: "scroll", y: 400 },
        ],
        probes: behaviorProbes,
    });
}
await writeFile(
    Bun.argv[2] ?? "calibration.json",
    JSON.stringify(suite, null, 2),
);
