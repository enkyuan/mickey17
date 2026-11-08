import { categories } from "./score";
const percent = (v: number | null | undefined) =>
    v == null ? "unmeasured" : `${(100 * v).toFixed(2)}%`;
/** Readable evidence index; machine report contains every fact and raw capture. */
export function markdownReport(r: any) {
    const lines = [
        `# ${r.name}`,
        "",
        `Status: **${r.status}**. Scope: declared cases only.`,
        "",
        `Track: ${r.generationMode}. CLI reconstruction executed: ${r.generationIncluded ? "yes" : "no"}. Reference: ${r.referenceKind}.`,
        "",
        `Scheduled runs: ${r.scheduledRuns}. Completed captures: ${r.results.filter((x: any) => !x.error).length}.`,
        "",
        "| Dimension | Required probes | Missing probes | Mean case match | Worst case match |",
        "| --- | ---: | ---: | ---: | ---: |",
    ];
    for (const c of categories) {
        const s = r.summary[c];
        lines.push(
            `| ${c} | ${s.required} | ${s.missing} | ${percent(s.macroAccuracy)} | ${percent(s.worstCaseAccuracy)} |`,
        );
    }
    const comparable = r.results.filter((x: any) => x.pixels?.comparable);
    lines.push(
        "",
        `Exact pixel comparison available for ${comparable.length}/${r.scheduledRuns} runs; ${comparable.filter((x: any) => x.pixels.changedPixels === 0).length} have zero changed pixels. These are screenshot matches, not whole-site accuracy.`,
        "",
        "## Qualification gates",
        "",
    );
    for (const [name, value] of Object.entries({
        referencePinned: r.referencePinned,
        referenceRepeatable: r.referenceRepeatable,
        sourceStable: r.sourceStable,
        buildStable: r.buildStable,
        calibrationPassed: r.calibrationPassed,
        sourceChecksPassed: r.sourceChecksPassed,
    }))
        lines.push(
            `- ${name}: ${value ? "passed" : "not passed / not applicable"}`,
        );
    if (r.sourceAudit) {
        const a = r.sourceAudit;
        lines.push(
            "",
            `React Doctor: ${a.reactDoctor}; ${a.doctorEvidence?.scanned ?? 0}/${a.files.length} files scanned; ${a.doctorEvidence?.errors ?? "unknown"} errors and ${a.doctorEvidence?.warnings ?? "unknown"} warnings.`,
            "",
            "AST findings, React Doctor diagnostics and build commands are in report.json and commands.json. They do not constitute a semantic code-quality rating.",
        );
    }
    if (r.fatal)
        lines.push(
            "",
            `Runner failure: ${String(r.fatal).replaceAll("\n", " ")}`,
        );
    lines.push(
        "",
        "## Coverage gaps",
        "",
        ...(r.coverageGaps.length
            ? r.coverageGaps.map((g: string) => `- ${g}`)
            : [
                  "No gaps declared. This does not establish coverage of unlisted states.",
              ]),
        "",
        "## Performance scope",
        "",
        "Frame samples are idle lab observations after actions. Navigation, transfer sizes, long tasks and heap estimates are raw observations in each capture. Cross-origin timing, caching, throttling, network and GPU effects limit comparison. No field Core Web Vitals, interaction-latency, or generalization claim is made.",
        "",
        "See report.json for source/build/suite/tool hashes and per-probe failures; raw DOM, screenshots, console and network evidence are retained alongside it.",
        "",
    );
    return lines.join("\n");
}
