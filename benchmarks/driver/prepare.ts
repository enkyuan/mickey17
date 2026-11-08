/** Add read-only page-error telemetry to an isolated copy of gstack's driver.
 * The installed driver, page scripts, CDP allowlist and permission checks are unchanged.
 */
import { cp, mkdir, readFile, writeFile, symlink } from "node:fs/promises";
import { resolve, join } from "node:path";
import { createHash } from "node:crypto";
const [sourceArgument, outputArgument] = Bun.argv.slice(2);
if (!sourceArgument || !outputArgument)
    throw Error(
        "Usage: bun driver/prepare.ts GSTACK_ROOT NEW_PRIVATE_DIRECTORY",
    );
const source = resolve(sourceArgument),
    output = resolve(outputArgument);
const original = await readFile(
    join(source, "browse/src/browser-manager.ts"),
    "utf8",
);
const anchor = "    page.on('console', (msg) => {";
if (original.split(anchor).length !== 2)
    throw Error(
        "Unsupported gstack driver; review the page event registration before adapting",
    );
const listener = `    // Mochi benchmark: uncaught exceptions are not console messages.
    page.on('pageerror', (error) => {
      addConsoleEntry({ timestamp: Date.now(), level: 'error', text: '[pageerror] ' + error.message });
    });

`;
const patched = original.includes("page.on('pageerror'")
    ? original
    : original.replace(anchor, listener + anchor);
await mkdir(output, { recursive: false });
await cp(join(source, "browse/src"), join(output, "browse/src"), {
    recursive: true,
});
await cp(join(source, "lib"), join(output, "lib"), { recursive: true });
await cp(join(source, "package.json"), join(output, "package.json"));
await symlink(join(source, "node_modules"), join(output, "node_modules"));
await writeFile(join(output, "browse/src/browser-manager.ts"), patched);
const hash = (s: string) => createHash("sha256").update(s).digest("hex");
await writeFile(
    join(output, "driver-provenance.json"),
    JSON.stringify(
        {
            source,
            version: JSON.parse(
                await readFile(join(source, "package.json"), "utf8"),
            ).version,
            originalSha256: hash(original),
            instrumentedSha256: hash(patched),
            change: "Read-only pageerror event logging; no page injection or permission changes",
        },
        null,
        2,
    ),
);
// JSON is emitted as data. A caller can use these values in its own process environment.
await writeFile(
    join(output, "environment.json"),
    JSON.stringify(
        {
            BROWSE_SERVER_SCRIPT: join(output, "browse/src/server.ts"),
            BROWSE_STATE_FILE: join(output, "state.json"),
            BROWSE_SESSION: "",
        },
        null,
        2,
    ),
);
console.log(join(output, "environment.json"));
