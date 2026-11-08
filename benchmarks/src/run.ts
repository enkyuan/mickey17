import { resolve, join } from "node:path";
import { parseArgs } from "node:util";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { fingerprint, hash } from "./fingerprint";
import { auditSource } from "./source-audit";
import { markdownReport } from "./report";
import { beginActionSampling, endActionSampling } from "./performance";
import { validateSuite, type Suite, type Case } from "./schema";
import { measurement } from "./measure";
import { doctorEvidence } from "./doctor";
import { acceptsCalibration } from "./calibration";
import { qualification } from "./qualification";
import {
    scoreFacts,
    pixels,
    summarize,
    distribution,
    type Fact,
} from "./score";
const { values: options } = parseArgs({
    args: Bun.argv.slice(2),
    strict: true,
    options: {
        suite: { type: "string" },
        browser: { type: "string" },
        out: { type: "string" },
        calibration: { type: "string" },
    },
});
const suitePath = options.suite,
    browser = options.browser,
    output = options.out;
if (!suitePath || !browser || !output)
    throw Error(
        "Usage: bun src/run.ts --suite suite.json --browser /path/to/gstack/browse --out .mochi/benchmarks/new-run",
    );
const suiteBytes = await readFile(resolve(suitePath)),
    suite: Suite = JSON.parse(suiteBytes.toString());
validateSuite(suite);
const out = resolve(output);
await mkdir(out, { recursive: false });
const startedAt = new Date().toISOString();
const benchmarkSource = await fingerprint(resolve(import.meta.dir, ".."));
const calibrationPath = options.calibration;
const calibration = calibrationPath
    ? JSON.parse(await readFile(resolve(calibrationPath), "utf8"))
    : null;
const browserSha256 = hash(await readFile(resolve(browser)));
const calibrationPassed = acceptsCalibration(calibration, browserSha256);
const commandLog: any[] = [];
async function command(
    argv: string[],
    cwd?: string,
    timeout = 60000,
    allowNonZero = false,
) {
    const start = performance.now(),
        p = Bun.spawn(argv, { cwd, stdout: "pipe", stderr: "pipe" });
    let timedOut = false;
    const timer = setTimeout(() => {
        timedOut = true;
        p.kill();
    }, timeout);
    const [stdout, stderr, code] = await Promise.all([
        new Response(p.stdout).text(),
        new Response(p.stderr).text(),
        p.exited,
    ]);
    clearTimeout(timer);
    commandLog.push({
        argv,
        cwd,
        code,
        timedOut,
        elapsedMs: performance.now() - start,
        stdout,
        stderr,
    });
    if ((code !== 0 && !allowNonZero) || timedOut)
        throw Error(`Command failed: ${argv[0]} ${argv[1] ?? ""} (${code})`);
    return stdout;
}
async function browse(...parts: string[]) {
    return command([resolve(browser!), ...parts]);
}
async function evaluate(expression: string, name: string) {
    const file = join(out, name + ".js"),
        result = join(out, name + ".json");
    await writeFile(file, expression);
    await browse("eval", file, "--out", result);
    return JSON.parse(await readFile(result, "utf8"));
}
async function capture(
    c: Case,
    side: "reference" | "candidate",
    repeat: number,
) {
    const label = `${c.id}.${side}.${repeat}`;
    await browse(
        "viewport",
        `${c.viewport[0]}x${c.viewport[1]}`,
        "--scale",
        "1",
    );
    await browse(
        "cdp",
        "Emulation.setEmulatedMedia",
        JSON.stringify({
            features: [
                {
                    name: "prefers-reduced-motion",
                    value: c.reducedMotion ? "reduce" : "no-preference",
                },
            ],
        }),
    );
    await browse("console", "--clear");
    await browse("network", "--clear");
    await browse(
        "goto",
        new URL(
            c.route,
            side === "reference"
                ? suite.referenceOrigin
                : suite.candidateOrigin,
        ).href,
    );
    await evaluate(
        `(async()=>{await document.fonts.ready;await new Promise(r=>setTimeout(r,${suite.referenceKind === "fixture" ? 50 : 3000}));return true})()`,
        label + ".ready",
    );
    if (c.actions.length)
        await evaluate(beginActionSampling, label + ".action-start");
    for (const action of c.actions) {
        if (action.kind === "scroll")
            await evaluate(
                `(async()=>{scrollTo(0,${JSON.stringify(action.y)});await new Promise(r=>setTimeout(r,1200));return scrollY})()`,
                label + ".scroll",
            );
        else if (action.kind === "press") await browse("press", action.key);
        else if (action.kind === "fill")
            await browse("fill", action[side], action.value);
        else if (action.kind === "focus")
            await evaluate(
                `(()=>{const e=document.querySelector(${JSON.stringify(action[side])});if(!e)throw Error('missing focus target');e.focus();if(document.activeElement!==e)throw Error('focus did not move');return true})()`,
                label + ".focus",
            );
        else await browse(action.kind, action[side]);
    }
    if (c.actions.length)
        await evaluate(
            "new Promise(r=>setTimeout(()=>r(true),1200))",
            label + ".action-settled",
        );
    const actionPerformance = c.actions.length
        ? await evaluate(endActionSampling, label + ".action-end")
        : null;
    const first = await evaluate(measurement(c.probes, side), label + ".first");
    if (
        first.context.viewport[0] !== c.viewport[0] ||
        first.context.viewport[1] !== c.viewport[1] ||
        first.context.reducedMotion !== c.reducedMotion ||
        new URL(first.context.url).pathname !== (c.expectedRoute ?? c.route) ||
        new URL(first.context.url).origin !==
            new URL(
                side === "reference"
                    ? suite.referenceOrigin
                    : suite.candidateOrigin,
            ).origin
    )
        throw Error("Browser context differs from case");
    const second = await evaluate(
        measurement(c.probes, side),
        label + ".measure",
    );
    const consoleLog = await browse("console", "--errors");
    await writeFile(join(out, label + ".console.txt"), consoleLog);
    const network = await browse("network");
    await writeFile(join(out, label + ".network.txt"), network);
    const screenshot = join(out, label + ".png");
    await browse("screenshot", "--viewport", screenshot);
    const dom = await browse("html", "html");
    await writeFile(join(out, label + ".html"), dom);
    return {
        ...second,
        actionPerformance,
        settled: JSON.stringify(first.facts) === JSON.stringify(second.facts),
        consoleErrors: consoleLog
            .split("\n")
            .filter((l) => l.includes("[error]")),
        htmlSha256: hash(dom),
        screenshot,
        networkSha256: hash(network),
    };
}
const results: any[] = [];
let fatal: string | undefined;
let sourceBefore: Awaited<ReturnType<typeof fingerprint>> | undefined;
let buildBefore: typeof sourceBefore;
let buildAfter: typeof sourceBefore;
let sourceAfter: typeof sourceBefore;
let referenceSource: typeof sourceBefore;
let referenceSourceAfter: typeof sourceBefore;
let sourceAudit: any = null;
try {
    const status = await browse("status");
    if (!/Tabs: 1\b/.test(status))
        throw Error(
            "Benchmark requires a dedicated one-tab browser; use a separate BROWSE_STATE_FILE",
        );
    if (suite.reconstruction)
        await command(
            suite.reconstruction.command,
            resolve(suite.reconstruction.cwd),
            300000,
        );
    if (suite.build)
        for (const step of suite.build)
            await command(step.command, resolve(step.cwd), 300000);
    if (suite.candidateBuild)
        buildBefore = await fingerprint(suite.candidateBuild);
    if (suite.sourceCapture)
        referenceSource = await fingerprint(suite.sourceCapture);
    if (suite.candidateProject) {
        sourceBefore = await fingerprint(suite.candidateProject);
        sourceAudit = {
            ...(await auditSource(
                suite.candidateProject,
                suite.requiredRoutes,
            )),
            reactDoctor: "not_run",
            buildChecks: suite.build?.length ?? 0,
        };
        const sourceFiles: string[] = sourceAudit.sourceFiles;
        if (suite.reactDoctor) {
            try {
                const raw = await command(
                    [
                        ...suite.reactDoctor.command,
                        ...sourceFiles.map((p) => resolve(p)),
                        "--json",
                        "--no-cache",
                        "--no-score",
                        "--no-supply-chain",
                    ],
                    resolve(suite.reactDoctor.cwd),
                    300000,
                    true,
                );
                sourceAudit.doctorEvidence = doctorEvidence(
                    JSON.parse(raw),
                    sourceFiles.length,
                );
                sourceAudit.reactDoctor = sourceAudit.doctorEvidence.passed
                    ? "passed"
                    : "diagnostics_or_incomplete";
            } catch {
                sourceAudit.reactDoctor = "failed_review_log";
            }
        }
    }
    for (const c of suite.cases)
        for (let repeat = 0; repeat < suite.repeats; repeat++) {
            const row: any = { case: c.id, repeat };
            try {
                row.reference = await capture(c, "reference", repeat);
                row.candidate = await capture(c, "candidate", repeat);
                const facts: Fact[] = c.probes.map((p) => ({
                    category: p.category,
                    tolerance: p.tolerance,
                    reference: row.reference.facts[p.id],
                    candidate: row.candidate.facts[p.id],
                    measured:
                        Object.hasOwn(row.reference.facts, p.id) &&
                        Object.hasOwn(row.candidate.facts, p.id),
                }));
                row.scores = scoreFacts(facts);
                row.pixels = pixels(
                    await readFile(row.reference.screenshot),
                    await readFile(row.candidate.screenshot),
                );
                row.failures = c.probes
                    .filter(
                        (p, i) => scoreFacts([facts[i]])[p.category].failed > 0,
                    )
                    .map((p) => ({
                        id: p.id,
                        reference: row.reference.facts[p.id] ?? null,
                        candidate: row.candidate.facts[p.id] ?? null,
                    }));
                row.performance = {
                    referenceIdleFramesMs: distribution(
                        row.reference.performance.frames,
                    ),
                    candidateIdleFramesMs: distribution(
                        row.candidate.performance.frames,
                    ),
                };
            } catch (error) {
                row.error = String(error);
                row.scores = scoreFacts(
                    c.probes.map((p) => ({
                        category: p.category,
                        reference: null,
                        candidate: null,
                        measured: false,
                    })),
                );
            }
            results.push(row);
            await writeFile(
                join(out, "progress.json"),
                JSON.stringify(results, null, 2),
            );
            console.log(
                `${c.id} repeat ${repeat + 1}/${suite.repeats}: ${row.error ?? `${row.failures.length} measured failures`}`,
            );
        }
    if (suite.candidateProject)
        sourceAfter = await fingerprint(suite.candidateProject);
    if (suite.candidateBuild)
        buildAfter = await fingerprint(suite.candidateBuild);
    if (suite.sourceCapture)
        referenceSourceAfter = await fingerprint(suite.sourceCapture);
} catch (error) {
    fatal = String(error);
} finally {
    await browse("cdp", "Emulation.setEmulatedMedia", '{"features":[]}').catch(
        () => {},
    );
    await writeFile(
        join(out, "commands.json"),
        JSON.stringify(commandLog, null, 2),
    );
}
// A failed run still contributes every scheduled probe to the denominator.
for (const c of suite.cases)
    for (let repeat = 0; repeat < suite.repeats; repeat++)
        if (!results.some((r) => r.case === c.id && r.repeat === repeat))
            results.push({
                case: c.id,
                repeat,
                error: "not executed",
                scores: scoreFacts(
                    c.probes.map((p) => ({
                        category: p.category,
                        reference: null,
                        candidate: null,
                        measured: false,
                    })),
                ),
            });
const unstable = results
    .filter(
        (r) =>
            (r.reference && !r.reference.settled) ||
            (r.candidate && !r.candidate.settled),
    )
    .map((r) => `${r.case}.${r.repeat}`);
const referenceNoise: any[] = [];
for (const c of suite.cases) {
    const runs = results.filter((r) => r.case === c.id && r.reference);
    for (const r of runs.slice(1))
        referenceNoise.push({
            case: c.id,
            repeat: r.repeat,
            factsStable:
                JSON.stringify(r.reference.facts) ===
                JSON.stringify(runs[0].reference.facts),
            pixels: pixels(
                await readFile(runs[0].reference.screenshot),
                await readFile(r.reference.screenshot),
            ),
        });
}
const passed = results.every(
    (r) =>
        !r.error &&
        !r.failures.length &&
        r.reference.settled &&
        r.candidate.settled &&
        !r.reference.consoleErrors.length &&
        !r.candidate.consoleErrors.length &&
        r.pixels.comparable &&
        r.pixels.changedPixels === 0,
);
const referencePinned =
    suite.referenceKind === "fixture" ||
    (suite.referenceKind === "captured-build" &&
        !!referenceSource &&
        referenceSource.sha256 === referenceSourceAfter?.sha256);
const allReferences = suite.cases.every(
    (c) =>
        results.filter((r) => r.case === c.id && r.reference).length ===
        suite.repeats,
);
const referenceRepeatable = allReferences
    ? referenceNoise.every(
          (r) =>
              r.factsStable &&
              r.pixels.comparable &&
              r.pixels.changedPixels === 0,
      )
    : null;
const sourceStable = sourceBefore
    ? sourceBefore.sha256 === sourceAfter?.sha256
    : null;
const buildStable = !!buildBefore && buildBefore.sha256 === buildAfter?.sha256;
const sourceChecksPassed =
    suite.generationMode === "fixture" ||
    (!!sourceAudit &&
        sourceAudit.motionDeclared &&
        !sourceAudit.directFramer &&
        !sourceAudit.framerImports.length &&
        !sourceAudit.recoveredImports.length &&
        !sourceAudit.opaqueFilenames.length &&
        !sourceAudit.findings.length &&
        sourceAudit.explicitAny === 0 &&
        sourceAudit.routeCoverage &&
        sourceAudit.buildChecks > 0 &&
        sourceAudit.reactDoctor === "passed" &&
        buildStable);
const outcome = qualification({
    fixture: suite.generationMode === "fixture",
    measurements: passed,
    sourceStable,
    buildStable,
    sourceChecks: sourceChecksPassed,
    calibration: calibrationPassed,
    referencePinned,
    referenceRepeatable,
    coverageGaps: suite.coverageGaps,
    fatal,
});
const report = {
    schemaVersion: 1,
    name: suite.name,
    startedAt,
    finishedAt: new Date().toISOString(),
    runtime: {
        bun: Bun.version,
        platform: process.platform,
        architecture: process.arch,
    },
    benchmarkSource,
    browserSha256,
    calibrationPassed,
    calibration,
    buildBefore,
    buildAfter,
    buildStable,
    sourceChecksPassed,
    status: outcome.status,
    qualificationFailures: outcome.reasons,
    performanceStatus: "uncontrolled_lab_observations",
    suiteSha256: hash(suiteBytes),
    generationMode: suite.generationMode,
    generationIncluded: !!suite.reconstruction,
    referenceKind: suite.referenceKind,
    referenceSource,
    referenceSourceAfter,
    referencePinned,
    referenceRepeatable,
    sourceBefore,
    sourceAfter,
    sourceStable,
    sourceAudit,
    coverageGaps: suite.coverageGaps,
    fatal,
    unstable,
    referenceNoise,
    scheduledRuns: suite.cases.length * suite.repeats,
    summary: summarize(
        Object.fromEntries(
            results.map((r) => [`${r.case}.${r.repeat}`, r.scores]),
        ),
    ),
    results,
    scope: "Measured benchmark cases only. No overall score. Category percentages include failed/missing probes. Reference instability, unmeasured behavior, source diagnostics and live-network variance are explicit. Frame intervals are synthetic lab samples, not field Core Web Vitals.",
};
await writeFile(join(out, "report.json"), JSON.stringify(report, null, 2));
await writeFile(join(out, "report.md"), markdownReport(report));
console.log(`Report: ${join(out, "report.json")} (${report.status})`);
if (report.status !== "matched_declared_benchmark") process.exitCode = 1;
