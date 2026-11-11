#!/usr/bin/env bun
// bun agent-flows/src/cli.ts --run <har>,<trace>[,<cookies>] --run ... [--ignore-host H]... [--ignore-path-prefix P]... [--only-traced] [--out graph.json]
// Exit code 0 when the flow is resolved, 2 when it is blocked, 1 on usage or input errors.

import { readFileSync, writeFileSync } from "node:fs";
import { infer, type Options } from "./classify.ts";
import { toJson } from "./graph.ts";
import { parseRecording } from "./har.ts";

function main(argv: string[]): number {
    const runs: string[][] = [];
    const opts: Required<Options> = { ignoreHosts: [], ignorePathPrefixes: [], onlyTraced: false };
    let out: string | null = null;
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i]!;
        const next = () => {
            const v = argv[++i];
            if (v === undefined) throw new Error(`${a} needs a value`);
            return v;
        };
        if (a === "--run") runs.push(next().split(","));
        else if (a === "--ignore-host") opts.ignoreHosts.push(next());
        else if (a === "--ignore-path-prefix") opts.ignorePathPrefixes.push(next());
        else if (a === "--only-traced") opts.onlyTraced = true;
        else if (a === "--out") out = next();
        else throw new Error(`unknown argument ${a}`);
    }
    if (runs.length < 2) throw new Error("at least two --run <har>,<trace>[,<cookies>] are required");
    const recs = runs.map(([har, trace, cookies]) => {
        if (!har || !trace) throw new Error("--run needs <har>,<trace>[,<cookies>]");
        return parseRecording(readFileSync(har, "utf8"), readFileSync(trace, "utf8"), cookies ? readFileSync(cookies, "utf8") : null);
    });
    const graph = infer(recs, opts);
    const json = toJson(graph);
    if (out) writeFileSync(out, json);
    else process.stdout.write(json);
    return graph.status === "resolved" ? 0 : 2;
}

try {
    process.exit(main(process.argv.slice(2)));
} catch (e) {
    console.error(e instanceof Error ? e.message : e);
    process.exit(1);
}
