// The unit tests need no setup. The React-in-a-DOM tests need an installed
// export (for its react / react-dom): MOCHI_COMPAT_EXPORTS=<dir>[:<dir>...].
import { describe, expect, test } from "bun:test";
import { resolve } from "node:path";
import { installOwnerGuard } from "../../src/export_template/src/lib/policy";

describe("installOwnerGuard", () => {
    test("dispatchers assigned later gain getOwner", () => {
        const internals: { A: unknown } = { A: null };
        installOwnerGuard(internals);
        const foreign: Record<string, unknown> = { getCacheForType: () => 1 };
        internals.A = foreign;
        expect(internals.A).toBe(foreign);
        expect(typeof foreign.getOwner).toBe("function");
        expect((foreign.getOwner as () => unknown)()).toBeNull();
    });
    test("an existing dispatcher is patched and real getOwner is kept", () => {
        const real = () => "owner";
        const own = { getOwner: real };
        const internals: { A: unknown } = { A: own };
        installOwnerGuard(internals);
        expect((internals.A as { getOwner: unknown }).getOwner).toBe(real);
        const bare: Record<string, unknown> = {};
        internals.A = bare;
        expect(typeof bare.getOwner).toBe("function");
        internals.A = null;
        expect(internals.A).toBeNull();
    });
    test("frozen dispatchers are wrapped, not mutated", () => {
        const internals: { A: unknown } = { A: null };
        installOwnerGuard(internals);
        const frozen = Object.freeze({ getCacheForType: () => 1 });
        internals.A = frozen;
        const seen = internals.A as { getOwner: () => unknown; getCacheForType: () => number };
        expect(seen).not.toBe(frozen);
        expect(seen.getOwner()).toBeNull();
        expect(seen.getCacheForType()).toBe(1);
    });
    test("is idempotent and tolerates missing internals", () => {
        const internals: { A: unknown } = { A: null };
        installOwnerGuard(internals);
        const descriptor = Object.getOwnPropertyDescriptor(internals, "A");
        installOwnerGuard(internals);
        expect(Object.getOwnPropertyDescriptor(internals, "A")).toEqual(descriptor);
        expect(() => installOwnerGuard(undefined)).not.toThrow();
        expect(() => installOwnerGuard(null)).not.toThrow();
    });
});

const exports = (process.env.MOCHI_COMPAT_EXPORTS ?? "").split(":").filter(Boolean);
async function run(dir: string, ...args: string[]) {
    const p = Bun.spawn(["bun", resolve(import.meta.dir, "compat-dev-react.ts"), dir, ...args], {
        env: { ...process.env, NODE_ENV: "development" },
        stdout: "pipe",
        stderr: "pipe",
    });
    const [out, err] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text(), p.exited]);
    const last = out.trim().split("\n").pop() ?? "";
    if (!last.startsWith("{")) throw new Error(`compat-dev-react.ts printed no result for ${dir}:\n${err || out}`);
    return JSON.parse(last) as { errors: string[]; html: string };
}
describe.skipIf(exports.length === 0)("React development JSX runtime with a foreign renderer", () => {
    for (const dir of exports) {
        test(`${dir}: without the guard the clash reproduces`, async () => {
            const result = await run(dir);
            expect(result.errors.join()).toContain("getOwner is not a function");
        }, 60_000);
        test(`${dir}: with the guard children render`, async () => {
            const result = await run(dir, "guard");
            expect(result.errors).toEqual([]);
            expect(result.html).toBe("<div><span>ok</span></div>");
        }, 60_000);
    }
});
