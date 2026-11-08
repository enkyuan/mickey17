import { categories, type Category } from "./score";
export type Probe = {
    id: string;
    category: Category;
    reference: string;
    candidate: string;
    property: string;
    tolerance?: number;
};
export type Action =
    | { kind: "scroll"; y: number }
    | {
          kind: "hover" | "click" | "focus";
          reference: string;
          candidate: string;
      }
    | { kind: "press"; key: string }
    | { kind: "fill"; reference: string; candidate: string; value: string };
export type Case = {
    id: string;
    route: string;
    expectedRoute?: string;
    viewport: [number, number];
    reducedMotion: boolean;
    actions: Action[];
    probes: Probe[];
};
export type Suite = {
    schemaVersion: 1;
    name: string;
    referenceOrigin: string;
    candidateOrigin: string;
    referenceKind: "live" | "captured-build" | "fixture";
    candidateProject?: string;
    generationMode:
        | "reviewed-authored-spec"
        | "compatibility"
        | "fixture"
        | "existing-project";
    sourceCapture?: string;
    candidateBuild?: string;
    reconstruction?: { command: string[]; cwd: string };
    build?: { command: string[]; cwd: string }[];
    coverageGaps: string[];
    requiredRoutes: string[];
    cases: Case[];
    repeats: number;
    reactDoctor?: { command: string[]; cwd: string };
};
const nonempty = (s: unknown): s is string =>
    typeof s === "string" && s.trim().length > 0;
const route = (s: unknown) =>
    nonempty(s) &&
    s.startsWith("/") &&
    !s.startsWith("//") &&
    !/[?#\\]/.test(s) &&
    new URL(s, "https://benchmark.test").pathname === s;
function onlyKeys(value: object, allowed: string[], context: string) {
    const unknown = Object.keys(value).filter((key) => !allowed.includes(key));
    if (unknown.length)
        throw Error(`Unsupported ${context} fields: ${unknown.join(", ")}`);
}
export function validateSuite(s: Suite) {
    onlyKeys(
        s,
        [
            "schemaVersion",
            "name",
            "referenceOrigin",
            "candidateOrigin",
            "referenceKind",
            "candidateProject",
            "generationMode",
            "sourceCapture",
            "candidateBuild",
            "reconstruction",
            "build",
            "coverageGaps",
            "requiredRoutes",
            "cases",
            "repeats",
            "reactDoctor",
        ],
        "suite",
    );
    if (
        s.schemaVersion !== 1 ||
        !nonempty(s.name) ||
        !Array.isArray(s.requiredRoutes) ||
        !s.requiredRoutes.length ||
        s.requiredRoutes.some((r) => !route(r)) ||
        new Set(s.requiredRoutes).size !== s.requiredRoutes.length ||
        !Array.isArray(s.cases) ||
        !s.cases.length ||
        s.cases.length > 4096 ||
        !Number.isInteger(s.repeats) ||
        s.repeats < 3 ||
        s.repeats > 10
    )
        throw Error(
            "Invalid suite: require unique routes, cases, and 3–10 integer repeats",
        );
    if (
        !["live", "captured-build", "fixture"].includes(s.referenceKind) ||
        ![
            "reviewed-authored-spec",
            "compatibility",
            "fixture",
            "existing-project",
        ].includes(s.generationMode) ||
        !Array.isArray(s.coverageGaps) ||
        s.coverageGaps.some((g) => !nonempty(g))
    )
        throw Error("Invalid provenance or coverage gaps");
    if (
        (s.referenceKind === "fixture" && s.generationMode !== "fixture") ||
        (s.generationMode === "fixture" && s.referenceKind !== "fixture")
    )
        throw Error("Fixture calibration must be a separate track");
    if (s.referenceKind === "captured-build" && !nonempty(s.sourceCapture))
        throw Error("Captured-build reference requires preserved input");
    if (s.generationMode !== "fixture" && !nonempty(s.candidateProject))
        throw Error("Candidate project is required");
    if (
        ["reviewed-authored-spec", "compatibility"].includes(
            s.generationMode,
        ) &&
        !s.reconstruction
    )
        throw Error("Tool-generation track requires a reconstruction command");
    for (const step of [
        ...(s.build ?? []),
        s.reconstruction,
        s.reactDoctor,
    ].filter(Boolean))
        if (
            !nonempty(step!.cwd) ||
            !Array.isArray(step!.command) ||
            !step!.command.length ||
            step!.command.some((a) => !nonempty(a))
        )
            throw Error("Invalid command");
    for (const origin of [s.referenceOrigin, s.candidateOrigin]) {
        const u = new URL(origin);
        if (
            !["http:", "https:"].includes(u.protocol) ||
            u.username ||
            u.password ||
            u.search ||
            u.hash ||
            u.pathname !== "/"
        )
            throw Error("Invalid benchmark origin");
    }
    const ids = new Set<string>();
    for (const c of s.cases) {
        onlyKeys(
            c,
            [
                "id",
                "route",
                "expectedRoute",
                "viewport",
                "reducedMotion",
                "actions",
                "probes",
            ],
            "case",
        );
        if (ids.has(c.id) || !/^[-a-z0-9.]+$/.test(c.id))
            throw Error("Duplicate/unsafe case id");
        ids.add(c.id);
        if (
            !s.requiredRoutes.includes(c.route) ||
            (c.expectedRoute !== undefined &&
                !s.requiredRoutes.includes(c.expectedRoute)) ||
            !Array.isArray(c.probes) ||
            !c.probes.length ||
            c.probes.length > 10000 ||
            !Array.isArray(c.viewport) ||
            c.viewport.length !== 2 ||
            c.viewport.some(
                (v) => !Number.isInteger(v) || v < 200 || v > 4096,
            ) ||
            typeof c.reducedMotion !== "boolean" ||
            !Array.isArray(c.actions)
        )
            throw Error("Invalid case coverage");
        for (const a of c.actions) {
            onlyKeys(
                a,
                a.kind === "scroll"
                    ? ["kind", "y"]
                    : a.kind === "press"
                      ? ["kind", "key"]
                      : a.kind === "fill"
                        ? ["kind", "reference", "candidate", "value"]
                        : ["kind", "reference", "candidate"],
                "action",
            );
            if (a.kind === "scroll") {
                if (!Number.isFinite(a.y) || a.y < 0)
                    throw Error("Invalid scroll");
            } else if (a.kind === "press") {
                if (!nonempty(a.key)) throw Error("Invalid key");
            } else if (a.kind === "fill") {
                if (
                    !nonempty(a.reference) ||
                    !nonempty(a.candidate) ||
                    typeof a.value !== "string"
                )
                    throw Error("Invalid fill");
            } else if (
                !["hover", "click", "focus"].includes(a.kind) ||
                !nonempty(a.reference) ||
                !nonempty(a.candidate)
            )
                throw Error("Invalid action");
        }
        const probes = new Set<string>();
        for (const p of c.probes) {
            onlyKeys(
                p,
                [
                    "id",
                    "category",
                    "reference",
                    "candidate",
                    "property",
                    "tolerance",
                ],
                "probe",
            );
            if (
                !nonempty(p.id) ||
                probes.has(p.id) ||
                !nonempty(p.reference) ||
                !nonempty(p.candidate) ||
                !nonempty(p.property) ||
                !categories.includes(p.category) ||
                (p.tolerance !== undefined &&
                    (!Number.isFinite(p.tolerance) ||
                        p.tolerance < 0 ||
                        p.tolerance > 1 ||
                        !p.property.startsWith("bounds.")))
            )
                throw Error("Duplicate/empty probe or non-geometric tolerance");
            probes.add(p.id);
        }
    }
    for (const path of s.requiredRoutes)
        for (const mobile of [true, false]) {
            if (
                !s.cases.some(
                    (c) =>
                        c.route === path &&
                        c.viewport[0] < 768 === mobile &&
                        !c.reducedMotion,
                )
            )
                throw Error(`Missing viewport for ${path}`);
            if (
                !s.cases.some(
                    (c) =>
                        c.route === path &&
                        c.viewport[0] < 768 === mobile &&
                        c.reducedMotion,
                )
            )
                throw Error(`Missing reduced-motion viewport for ${path}`);
        }
}
