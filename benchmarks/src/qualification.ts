/** Unknown evidence is never a passing gate. Fixtures explicitly omit source checks. */
export type Gate = boolean | null;
export function qualification(input: {
    fixture: boolean;
    measurements: boolean;
    sourceStable: Gate;
    buildStable: Gate;
    sourceChecks: Gate;
    calibration: Gate;
    referencePinned: Gate;
    referenceRepeatable: Gate;
    coverageGaps: string[];
    fatal?: string;
}) {
    const reasons = [...input.coverageGaps];
    if (input.fatal) reasons.push(input.fatal);
    for (const key of [
        "measurements",
        "referencePinned",
        "referenceRepeatable",
    ] as const)
        if (input[key] !== true)
            reasons.push(
                `${key}: ${input[key] === null ? "unmeasured" : "failed"}`,
            );
    if (!input.fixture)
        for (const key of [
            "sourceStable",
            "buildStable",
            "sourceChecks",
            "calibration",
        ] as const)
            if (input[key] !== true)
                reasons.push(
                    `${key}: ${input[key] === null ? "unmeasured" : "failed"}`,
                );
    // A fixture which does supply source still cannot conceal changes to it.
    if (input.fixture && input.sourceStable === false)
        reasons.push("sourceStable: failed");
    return {
        status: reasons.length ? "not_qualified" : "matched_declared_benchmark",
        reasons,
    };
}
