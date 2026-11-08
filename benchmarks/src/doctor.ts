/** No server-derived score is treated as runtime performance or parity evidence. */
export function doctorEvidence(report: any, expectedFiles: number) {
    const projects = Array.isArray(report?.projects) ? report.projects : [];
    const scanned = projects.reduce(
        (n: number, p: any) =>
            n +
            (typeof p.scannedFileCount === "number" ? p.scannedFileCount : 0),
        0,
    );
    const complete =
        expectedFiles > 0 &&
        scanned >= expectedFiles &&
        projects.length > 0 &&
        projects.every(
            (p: any) =>
                p.complete === true &&
                Array.isArray(p.skippedChecks) &&
                p.skippedChecks.length === 0,
        );
    return {
        version: report?.version ?? null,
        scanned,
        expectedFiles,
        complete,
        errors: report?.summary?.errorCount ?? null,
        warnings: report?.summary?.warningCount ?? null,
        diagnostics: report?.diagnostics ?? [],
        passed:
            complete &&
            report?.summary?.errorCount === 0 &&
            report?.summary?.warningCount === 0,
    };
}
