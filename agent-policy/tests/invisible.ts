import { expect } from "bun:test";
import { CLOSE, LINE, OPEN, PAGE_RULE, unwrapPageText } from "../src/wrap.ts";

/** Characters that must never survive in text handed to an agent. */
export const INVISIBLE_RE = /[\p{Cf}\p{Cc}\p{Co}\p{Cn}\p{Cs}]/u;
const INVISIBLE = INVISIBLE_RE;

export function assertWrapInvariants(block: string, ref = "node_0123abcd"): void {
    const lines = block.split("\n");
    expect(lines.length).toBeGreaterThanOrEqual(4);
    expect(lines[0]!.startsWith(OPEN)).toBe(true);
    expect(lines[1]).toBe(PAGE_RULE);
    expect(lines.at(-1)!.startsWith(CLOSE)).toBe(true);
    expect(lines.filter((l) => l.startsWith(OPEN)).length).toBe(1);
    expect(lines.filter((l) => l.startsWith(CLOSE)).length).toBe(1);
    for (const l of lines.slice(2, -1)) {
        expect(l.startsWith(LINE), "content line without prefix").toBe(true);
        expect(/[<>\[\]`]/.test(l), "raw markup character inside content").toBe(false);
    }
    expect(block.replace(/[\n\t]/g, "")).not.toMatch(INVISIBLE);
    expect(unwrapPageText(block)).not.toBeNull();
    expect(block).toContain(`ref=${ref}`);
}

