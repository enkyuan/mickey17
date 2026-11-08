import { test, expect } from "bun:test";
import { inspectSource } from "../src/source-audit";
import { fingerprint } from "../src/fingerprint";
import { mkdtemp, writeFile, rm, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
test("Motion audit inspects syntax rather than unrelated strings or comments", () => {
    const result = inspectSource(
        "motion.tsx",
        `// import { motion } from 'framer-motion';
 const advice="framer-motion"; import {m} from 'motion/react-m';
 export {animate} from 'motion'; const lazy=import('framer-motion/m');`,
    );
    expect(result.imports).toEqual([
        "motion/react-m",
        "motion",
        "framer-motion/m",
    ]);
});
test("source evidence inventories routes, unsafe evaluation and explicit any", () => {
    const r = inspectSource(
        "index.tsx",
        `export const Route=createFileRoute('/catalog/')({});
 export function Card({text}:{text:any}){return <div dangerouslySetInnerHTML={{__html:text}}/>}
 const f=new Function('return 1');`,
    );
    expect(r.routes).toEqual(["/catalog/"]);
    expect(r.explicitAny).toBe(1);
    expect(r.findings.map((f) => f.kind)).toEqual([
        "raw-html-review-required",
        "runtime-code-evaluation",
    ]);
});
test("fingerprint catches new/deleted/changed files and rejects symlinks", async () => {
    const root = await mkdtemp(join(tmpdir(), "mochi-benchmark-"));
    try {
        await writeFile(join(root, "source.ts"), "export const value=1");
        const a = await fingerprint(root);
        await writeFile(join(root, "extra.ts"), "export const extra=2");
        const b = await fingerprint(root);
        expect(b.sha256).not.toBe(a.sha256);
        await rm(join(root, "extra.ts"));
        expect((await fingerprint(root)).sha256).toBe(a.sha256);
        await writeFile(join(root, "source.ts"), "export const value=2");
        expect((await fingerprint(root)).sha256).not.toBe(a.sha256);
        await symlink(join(root, "source.ts"), join(root, "alias.ts"));
        await expect(fingerprint(root)).rejects.toThrow("Symlink");
    } finally {
        await rm(root, { recursive: true, force: true });
    }
});
