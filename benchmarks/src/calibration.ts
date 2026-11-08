import { hash } from "./fingerprint";
export const requiredMutations = [
    "layout",
    "typography",
    "styling",
    "content",
    "assets",
    "graphics",
    "behavior",
    "routing",
    "overlap",
    "console",
] as const;
export function calibrationEvidence(
    control: any,
    mutations: any,
    browserSha256: string,
) {
    const misses = requiredMutations.filter(
        (name) =>
            !Array.isArray(mutations?.mutations) ||
            mutations.mutations.filter(
                (m: any) => m.mutation === name && m.detected === true,
            ).length !== 1,
    );
    const controlValid =
        control?.generationMode === "fixture" &&
        control?.referenceKind === "fixture" &&
        control?.status === "matched_declared_benchmark" &&
        control?.scheduledRuns >= 24 &&
        control?.referenceRepeatable === true;
    const sameDriver =
        control?.browserSha256 === browserSha256 &&
        mutations?.browserSha256 === browserSha256;
    const baselineClean =
        typeof mutations?.baselineErrors === "string" &&
        !mutations.baselineErrors.includes("[error]");
    return {
        schemaVersion: 1,
        kind: "benchmark-calibration",
        browserSha256,
        controlSha256: hash(JSON.stringify(control)),
        mutationsSha256: hash(JSON.stringify(mutations)),
        controlValid,
        sameDriver,
        baselineClean,
        misses,
        passed:
            controlValid && sameDriver && baselineClean && misses.length === 0,
    };
}
export function acceptsCalibration(c: any, browserSha256: string) {
    return (
        c?.schemaVersion === 1 &&
        c?.kind === "benchmark-calibration" &&
        c?.browserSha256 === browserSha256 &&
        c?.passed === true &&
        c?.controlValid === true &&
        c?.sameDriver === true &&
        c?.baselineClean === true &&
        Array.isArray(c?.misses) &&
        c.misses.length === 0
    );
}
