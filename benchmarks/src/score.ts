import { PNG } from "pngjs";
export const categories = [
    "layout",
    "typography",
    "styling",
    "content",
    "assets",
    "graphics",
    "behavior",
    "routing",
] as const;
export type Category = (typeof categories)[number];
export type Fact = {
    category: Category;
    tolerance?: number;
    reference: unknown;
    candidate: unknown;
    measured: boolean;
};
export type CategoryScore = {
    required: number;
    measured: number;
    passed: number;
    failed: number;
    coverage: number | null;
    accuracy: number | null;
    maxAbsoluteError: number | null;
    meanAbsoluteError: number | null;
};
export function scoreFacts(facts: Fact[]): Record<Category, CategoryScore> {
    return Object.fromEntries(
        categories.map((category) => {
            const group = facts.filter((f) => f.category === category),
                measured = group.filter(
                    (f) =>
                        f.measured &&
                        [f.reference, f.candidate].every(
                            (v) =>
                                typeof v === "string" ||
                                typeof v === "boolean" ||
                                (typeof v === "number" && Number.isFinite(v)),
                        ),
                ),
                errors: number[] = [];
            let passed = 0;
            for (const f of measured) {
                const tolerance = f.tolerance ?? 0;
                if (
                    !Number.isFinite(tolerance) ||
                    tolerance < 0 ||
                    tolerance > 1
                )
                    throw new Error(
                        "Tolerance must be between zero and one; use zero for non-pixel facts",
                    );
                if (
                    typeof f.reference === "number" &&
                    typeof f.candidate === "number"
                ) {
                    if (
                        !Number.isFinite(f.reference) ||
                        !Number.isFinite(f.candidate)
                    )
                        continue;
                    const error = Math.abs(f.reference - f.candidate);
                    errors.push(error);
                    if (error <= tolerance) passed++;
                } else if (
                    ["string", "boolean"].includes(typeof f.reference) &&
                    f.reference === f.candidate
                )
                    passed++;
            }
            return [
                category,
                {
                    required: group.length,
                    measured: measured.length,
                    passed,
                    failed: group.length - passed,
                    coverage: group.length
                        ? measured.length / group.length
                        : null,
                    accuracy: group.length ? passed / group.length : null,
                    maxAbsoluteError: errors.length
                        ? Math.max(...errors)
                        : null,
                    meanAbsoluteError: errors.length
                        ? errors.reduce((a, b) => a + b, 0) / errors.length
                        : null,
                },
            ];
        }),
    ) as Record<Category, CategoryScore>;
}
export function pixels(reference: Uint8Array, candidate: Uint8Array) {
    const a = PNG.sync.read(Buffer.from(reference)),
        b = PNG.sync.read(Buffer.from(candidate));
    if (a.width !== b.width || a.height !== b.height)
        return {
            comparable: false,
            reason: "dimensions differ",
            reference: [a.width, a.height],
            candidate: [b.width, b.height],
        };
    let changed = 0,
        sum = 0,
        max = 0;
    for (let i = 0; i < a.data.length; i += 4) {
        let delta = 0;
        for (let c = 0; c < 4; c++) {
            const d = Math.abs(a.data[i + c] - b.data[i + c]);
            sum += d;
            max = Math.max(max, d);
            delta = Math.max(delta, d);
        }
        if (delta > 0) changed++;
    }
    return {
        comparable: true,
        width: a.width,
        height: a.height,
        changedPixels: changed,
        totalPixels: a.width * a.height,
        exactPixelMatch: 1 - changed / (a.width * a.height),
        meanChannelError: sum / a.data.length,
        maxChannelError: max,
    };
}
export function summarize(
    cases: Record<string, Record<Category, CategoryScore>>,
) {
    return Object.fromEntries(
        categories.map((category) => {
            const values = Object.values(cases).map((c) => c[category]);
            const scored = values.filter((v) => v.required > 0);
            return [
                category,
                {
                    cases: values.length,
                    coveredCases: scored.length,
                    macroAccuracy: scored.length
                        ? scored.reduce((s, v) => s + v.accuracy!, 0) /
                          scored.length
                        : null,
                    worstCaseAccuracy: scored.length
                        ? Math.min(...scored.map((v) => v.accuracy!))
                        : null,
                    required: values.reduce((s, v) => s + v.required, 0),
                    missing: values.reduce(
                        (s, v) => s + v.required - v.measured,
                        0,
                    ),
                },
            ];
        }),
    );
}
export function distribution(values: number[]) {
    if (!values.length)
        return { samples: 0, median: null, p95: null, worst: null };
    const sorted = [...values].sort((a, b) => a - b);
    return {
        samples: values.length,
        median:
            sorted.length % 2
                ? sorted[Math.floor(sorted.length / 2)]
                : (sorted[sorted.length / 2 - 1] + sorted[sorted.length / 2]) /
                  2,
        p95: sorted[Math.ceil(sorted.length * 0.95) - 1],
        worst: sorted.at(-1),
    };
}
