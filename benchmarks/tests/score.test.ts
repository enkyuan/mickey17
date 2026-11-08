import { describe, test, expect } from "bun:test";
import { PNG } from "pngjs";
import {
    scoreFacts,
    summarize,
    pixels,
    distribution,
    type Fact,
} from "../src/score";
import { validateSuite } from "../src/schema";
const fact = (overrides: Partial<Fact> = {}): Fact => ({
    category: "layout",
    reference: 100,
    candidate: 100,
    measured: true,
    ...overrides,
});
describe("honest denominators", () => {
    test("missing and failing probes remain failures", () => {
        const x = scoreFacts([
            fact(),
            fact({ measured: false }),
            fact({ candidate: 101 }),
        ]).layout;
        expect(x.required).toBe(3);
        expect(x.accuracy).toBe(1 / 3);
        expect(x.coverage).toBe(2 / 3);
        expect(x.maxAbsoluteError).toBe(1);
    });
    test("empty categories are unmeasured, never 100%", () =>
        expect(scoreFacts([]).graphics.accuracy).toBeNull());
    test("tiny numeric discrepancies fail with zero tolerance", () =>
        expect(
            scoreFacts([fact({ candidate: 100.000001 })]).layout.failed,
        ).toBe(1));
    test("bounded explicit pixel tolerance", () => {
        expect(
            scoreFacts([fact({ candidate: 100.5, tolerance: 0.5 })]).layout
                .passed,
        ).toBe(1);
        expect(() => scoreFacts([fact({ tolerance: 10 })])).toThrow();
    });
    test("null, infinity, missing and equal objects never count as accuracy", () => {
        for (const v of [null, undefined, Infinity, {}, []])
            expect(
                scoreFacts([fact({ reference: v, candidate: v })]).layout
                    .failed,
            ).toBe(1);
    });
    test("large easy page cannot hide small failed page", () => {
        const a = scoreFacts(Array.from({ length: 1000 }, () => fact())),
            b = scoreFacts([fact({ candidate: 0 })]);
        const s = summarize({ a, b }).layout;
        expect(s.macroAccuracy).toBe(0.5);
        expect(s.worstCaseAccuracy).toBe(0);
    });
    test("failed selector is included in coverage loss", () =>
        expect(
            scoreFacts([
                fact({
                    reference: undefined,
                    candidate: undefined,
                    measured: false,
                }),
            ]).layout.coverage,
        ).toBe(0));
});
function png(width: number, height: number, red = 0) {
    const p = new PNG({ width, height });
    for (let i = 0; i < p.data.length; i += 4) {
        p.data[i] = red;
        p.data[i + 3] = 255;
    }
    return PNG.sync.write(p);
}
describe("pixel evidence", () => {
    test("identical control passes", () =>
        expect(pixels(png(2, 2), png(2, 2))).toMatchObject({
            exactPixelMatch: 1,
            changedPixels: 0,
        }));
    test("one channel step is reported, not discarded", () =>
        expect(pixels(png(2, 2), png(2, 2, 1))).toMatchObject({
            exactPixelMatch: 0,
            changedPixels: 4,
            maxChannelError: 1,
        }));
    test("dimension mismatch cannot score", () =>
        expect(pixels(png(2, 2), png(3, 2)).comparable).toBe(false));
});
test("performance distribution preserves worst sample", () =>
    expect(distribution([1, 2, 3, 100])).toEqual({
        samples: 4,
        median: 2.5,
        p95: 100,
        worst: 100,
    }));
test("suite cannot omit mobile, reduced motion or repeat sampling", () => {
    expect(() =>
        validateSuite({
            schemaVersion: 1,
            name: "bad",
            requiredRoutes: ["/"],
            cases: [],
            repeats: 1,
        } as any),
    ).toThrow();
});
import { doctorEvidence } from "../src/doctor";
describe("React Doctor coverage", () => {
    test("zero-file high score cannot qualify", () =>
        expect(
            doctorEvidence(
                {
                    summary: { score: 100, errorCount: 0, warningCount: 0 },
                    projects: [
                        {
                            scannedFileCount: 0,
                            complete: true,
                            skippedChecks: [],
                        },
                    ],
                },
                50,
            ).passed,
        ).toBe(false));
    test("partial scan cannot qualify", () =>
        expect(
            doctorEvidence(
                {
                    summary: { errorCount: 0, warningCount: 0 },
                    projects: [
                        {
                            scannedFileCount: 49,
                            complete: true,
                            skippedChecks: [],
                        },
                    ],
                },
                50,
            ).passed,
        ).toBe(false));
    test("diagnostics remain visible even if tool exit code was zero", () =>
        expect(
            doctorEvidence(
                {
                    summary: { errorCount: 2, warningCount: 1 },
                    projects: [
                        {
                            scannedFileCount: 50,
                            complete: true,
                            skippedChecks: [],
                        },
                    ],
                },
                50,
            ),
        ).toMatchObject({
            complete: true,
            passed: false,
            errors: 2,
            warnings: 1,
        }));
    test("complete clean scan qualifies only source diagnostics", () =>
        expect(
            doctorEvidence(
                {
                    summary: { errorCount: 0, warningCount: 0 },
                    projects: [
                        {
                            scannedFileCount: 50,
                            complete: true,
                            skippedChecks: [],
                        },
                    ],
                },
                50,
            ).passed,
        ).toBe(true));
});
const validSuite = (): any => ({
    schemaVersion: 1,
    name: "fixture",
    referenceOrigin: "http://localhost:4197",
    candidateOrigin: "http://localhost:4198",
    referenceKind: "fixture",
    generationMode: "fixture",
    coverageGaps: [],
    requiredRoutes: ["/"],
    repeats: 3,
    cases: [false, true].flatMap((reducedMotion) =>
        [390, 1440].map((w) => ({
            id: `${w}.${reducedMotion}`,
            route: "/",
            viewport: [w, 900],
            reducedMotion,
            actions: [],
            probes: [
                {
                    id: "x",
                    category: "layout",
                    property: "bounds.x",
                    reference: "h1",
                    candidate: "h1",
                },
            ],
        })),
    ),
});
test("strict suite validation rejects ambiguous denominators and mislabelled generation", () => {
    for (const mutate of [
        (s: any) => (s.repeats = 3.5),
        (s: any) => (s.repeats = NaN),
        (s: any) => (s.cases[0].viewport = [390]),
        (s: any) => (s.cases[0].viewport[1] = NaN),
        (s: any) => (s.cases[0].actions = [{ kind: "scroll", y: NaN }]),
        (s: any) => (s.generationMode = "reviewed-authored-spec"),
        (s: any) =>
            (s.cases[0].probes[0] = {
                ...s.cases[0].probes[0],
                property: "children.count",
                tolerance: 1,
            }),
        (s: any) => (s.requiredRoutes = ["/", "/"]),
        (s: any) => (s.referenceOrigin = "https://example.test/path"),
    ]) {
        const s = validSuite();
        mutate(s);
        expect(() => validateSuite(s)).toThrow();
    }
    expect(() => validateSuite(validSuite())).not.toThrow();
});

test("invalid values cannot count as measured coverage", () => {
    expect(
        scoreFacts([fact({ reference: null, candidate: null })]).layout
            .coverage,
    ).toBe(0);
});
import {
    calibrationEvidence,
    acceptsCalibration,
    requiredMutations,
} from "../src/calibration";
test("calibration requires clean control, all mutations and the same driver", () => {
    const control = {
        generationMode: "fixture",
        referenceKind: "fixture",
        status: "matched_declared_benchmark",
        scheduledRuns: 24,
        referenceRepeatable: true,
        browserSha256: "driver",
    };
    const mutations = {
        browserSha256: "driver",
        baselineErrors: "(no console errors)",
        mutations: requiredMutations.map((mutation) => ({
            mutation,
            detected: true,
        })),
    };
    expect(
        acceptsCalibration(
            calibrationEvidence(control, mutations, "driver"),
            "driver",
        ),
    ).toBe(true);
    expect(
        calibrationEvidence(
            { ...control, status: "not_qualified" },
            mutations,
            "driver",
        ).passed,
    ).toBe(false);
    expect(
        calibrationEvidence(
            control,
            { ...mutations, mutations: mutations.mutations.slice(0, -1) },
            "driver",
        ).passed,
    ).toBe(false);
    expect(
        calibrationEvidence(control, mutations, "different-driver").passed,
    ).toBe(false);
    expect(
        calibrationEvidence(
            control,
            { ...mutations, baselineErrors: "[error] problem" },
            "driver",
        ).passed,
    ).toBe(false);
    expect(acceptsCalibration({ passed: true }, "driver")).toBe(false);
});

test("unsupported clocks and silently ignored options cannot qualify", () => {
    const s = validSuite();
    s.cases[0].graphicsClock = "pretend-synchronized";
    expect(() => validateSuite(s)).toThrow("Unsupported case fields");
    delete s.cases[0].graphicsClock;
    s.threshold = 0.5;
    expect(() => validateSuite(s)).toThrow("Unsupported suite fields");
});

import { qualification } from "../src/qualification";
test("qualification never promotes partial, stale or unmeasured evidence", () => {
    const passing = {
        fixture: false,
        measurements: true,
        sourceStable: true,
        buildStable: true,
        sourceChecks: true,
        calibration: true,
        referencePinned: true,
        referenceRepeatable: true,
        coverageGaps: [],
    };
    expect(qualification(passing).status).toBe("matched_declared_benchmark");
    for (const gate of [
        "sourceStable",
        "buildStable",
        "sourceChecks",
        "calibration",
        "referencePinned",
        "referenceRepeatable",
    ]) {
        expect(qualification({ ...passing, [gate]: null }).status).toBe(
            "not_qualified",
        );
        expect(qualification({ ...passing, [gate]: false }).status).toBe(
            "not_qualified",
        );
    }
    expect(
        qualification({ ...passing, coverageGaps: ["missing shader clock"] })
            .status,
    ).toBe("not_qualified");
    expect(
        qualification({ ...passing, fatal: "navigation failed" }).status,
    ).toBe("not_qualified");
    expect(
        qualification({
            ...passing,
            fixture: true,
            sourceStable: null,
            buildStable: null,
            sourceChecks: null,
            calibration: null,
        }).status,
    ).toBe("matched_declared_benchmark");
});
