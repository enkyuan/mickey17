// Executable calibration: intentionally break each measured dimension and require detection.
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { resolve, join } from "node:path";
import { hash } from "../src/fingerprint";
import { measurement } from "../src/measure";
import { scoreFacts, type Category } from "../src/score";
import type { Suite } from "../src/schema";
const [browserPath, suitePath, outPath] = Bun.argv.slice(2);
if (!browserPath || !suitePath || !outPath)
    throw Error(
        "Usage: bun tests/browser-mutations.ts BROWSE CALIBRATION_SUITE OUTPUT_DIR",
    );
const browser = resolve(browserPath),
    out = resolve(outPath),
    suite: Suite = JSON.parse(await readFile(suitePath, "utf8"));
await mkdir(out, { recursive: false });
async function run(...args: string[]) {
    const p = Bun.spawn([browser, ...args], { stdout: "pipe", stderr: "pipe" });
    const [stdout, stderr, exit] = await Promise.all([
        new Response(p.stdout).text(),
        new Response(p.stderr).text(),
        p.exited,
    ]);
    if (exit) throw Error(stderr || stdout);
    return stdout;
}
const probes = suite.cases[0].probes;
async function capture(name: string) {
    await run("viewport", "1440x900");
    await run("console", "--clear");
    await run("goto", `http://127.0.0.1:4198/?mutation=${name}`);
    await run("click", "#menu-trigger");
    const file = join(out, name + ".js"),
        json = join(out, name + ".json");
    await writeFile(file, measurement(probes, "candidate"));
    await run("eval", file, "--out", json);
    return {
        data: JSON.parse(await readFile(json, "utf8")),
        console: await run("console", "--errors"),
    };
}
const baseline = await capture("none"),
    results = [];
for (const mutation of [
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
]) {
    const sample = await capture(mutation),
        scores = scoreFacts(
            probes.map((p) => ({
                category: p.category,
                reference: baseline.data.facts[p.id],
                candidate: sample.data.facts[p.id],
                measured:
                    Object.hasOwn(sample.data.facts, p.id) &&
                    Object.hasOwn(baseline.data.facts, p.id),
            })),
        );
    const category = (mutation === "overlap" ? "layout" : mutation) as Category;
    const detected =
        mutation === "console"
            ? sample.console.includes("Calibration runtime regression")
            : scores[category].failed > 0;
    results.push({ mutation, detected, scores, console: sample.console });
    console.log(`${mutation}: ${detected ? "detected" : "MISSED"}`);
}
await writeFile(
    join(out, "report.json"),
    JSON.stringify(
        {
            browserSha256: hash(await readFile(browser)),
            baselineErrors: baseline.console,
            mutations: results,
        },
        null,
        2,
    ),
);
if (results.some((r) => !r.detected))
    throw Error("Benchmark missed an injected regression");
