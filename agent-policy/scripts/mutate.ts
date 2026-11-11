// Applies each mutation to a throwaway copy of the package and requires `bun test` to fail.
// Usage: bun scripts/mutate.ts [--only id,id] [--jobs N]
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { MUTATIONS } from "./mutations.ts";

const root = resolve(import.meta.dir, "..");
const args = process.argv.slice(2);
const only = args.includes("--only") ? new Set(args[args.indexOf("--only") + 1]!.split(",")) : null;
const jobs = args.includes("--jobs") ? Number(args[args.indexOf("--jobs") + 1]) : 4;

const work = mkdtempSync(join(tmpdir(), "agent-policy-mut-"));

function fresh(name: string): string {
    const dir = join(work, name);
    for (const d of ["src", "tests", "fixtures", "scripts"]) cpSync(join(root, d), join(dir, d), { recursive: true });
    for (const f of ["package.json", "tsconfig.json"]) cpSync(join(root, f), join(dir, f));
    symlinkSync(join(root, "node_modules"), join(dir, "node_modules"));
    // The guard test reads the mutated source and would "kill" every mutant by itself.
    rmSync(join(dir, "tests/mutation-guard.test.ts"), { force: true });
    return dir;
}

async function runTests(dir: string): Promise<{ code: number; failed: string[] }> {
    const p = Bun.spawn(["bun", "test"], { cwd: dir, stdout: "pipe", stderr: "pipe", env: { ...process.env, NO_COLOR: "1" } });
    const [out, err] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text()]);
    const code = await p.exited;
    const failed = [...(out + err).matchAll(/^\(fail\) (.+?)(?: \[[\d.]+ms\])?$/gm)].map((m) => m[1]!);
    return { code, failed };
}

const baselineDir = fresh("baseline");
const base = await runTests(baselineDir);
if (base.code !== 0) {
    console.error("baseline is red; refusing to mutate", base.failed.slice(0, 5));
    process.exit(2);
}

const list = MUTATIONS.filter((m) => !only || only.has(m.id));
const results: { id: string; killed: boolean; by: string }[] = [];
let next = 0;

async function worker(n: number): Promise<void> {
    for (;;) {
        const i = next++;
        const m = list[i];
        if (!m) return;
        const dir = fresh(`m${n}-${i}`);
        const path = join(dir, "src", m.file);
        const text = readFileSync(path, "utf8");
        const count = text.split(m.find).length - 1;
        if (count !== 1) {
            results.push({ id: m.id, killed: false, by: `INVALID: find string occurs ${count} times` });
            continue;
        }
        writeFileSync(path, text.replace(m.find, () => m.replace));
        const r = await runTests(dir);
        results.push({ id: m.id, killed: r.code !== 0, by: r.failed[0] ?? (r.code !== 0 ? "(crash / compile error)" : "") });
        rmSync(dir, { recursive: true, force: true });
    }
}

await Promise.all(Array.from({ length: jobs }, (_, n) => worker(n)));
rmSync(work, { recursive: true, force: true });

results.sort((a, b) => MUTATIONS.findIndex((m) => m.id === a.id) - MUTATIONS.findIndex((m) => m.id === b.id));
for (const r of results) console.log(`${r.killed ? "killed  " : "SURVIVED"} ${r.id.padEnd(34)} ${r.killed ? r.by.slice(0, 90) : r.by}`);
const survived = results.filter((r) => !r.killed);
console.log(`\n${results.length - survived.length}/${results.length} mutants killed`);
if (existsSync(join(root, "node_modules")) && survived.length > 0) process.exit(1);
