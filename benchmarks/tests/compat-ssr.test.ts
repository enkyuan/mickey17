// Runs compat-ssr.ts against built compatibility exports. Building an export
// needs the TanStack toolchain (bun install, vite build), so the exports are
// supplied rather than built here:
//
//   MOCHI_COMPAT_EXPORTS=/path/to/natural:/path/to/garden bun test tests/compat-ssr.test.ts
//
// Each directory must already pass `bun install --frozen-lockfile && bun run check
// && bun run build`. Without the variable the suite is skipped, never green.
import { describe, expect, test } from "bun:test";
import { resolve } from "node:path";

const exports = (process.env.MOCHI_COMPAT_EXPORTS ?? "").split(":").filter(Boolean);
describe.skipIf(exports.length === 0)("compat export server rendering", () => {
    for (const dir of exports)
        test(dir, async () => {
            const process_ = Bun.spawn(["bun", resolve(import.meta.dir, "compat-ssr.ts"), dir], {
                stdout: "pipe",
                stderr: "pipe",
            });
            const [stdout, stderr, code] = await Promise.all([
                new Response(process_.stdout).text(),
                new Response(process_.stderr).text(),
                process_.exited,
            ]);
            expect(stderr + stdout).not.toContain("FAIL");
            expect(code).toBe(0);
        }, 120_000);
});
