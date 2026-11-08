import { createHash } from "node:crypto";
import { readdir, readFile } from "node:fs/promises";
import { resolve, join } from "node:path";
export const hash = (b: string | Uint8Array) =>
    createHash("sha256").update(b).digest("hex");
/** Stable file inventory, including newly added files. Build output is hashed separately. */
export async function fingerprint(
    root: string,
): Promise<{
    sha256: string;
    files: number;
    bytes: number;
    inventory: Record<string, string>;
}> {
    const entries: Record<string, string> = {};
    let bytes = 0;
    async function walk(dir: string, prefix = "") {
        for (const e of (await readdir(dir, { withFileTypes: true })).sort(
            (a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0),
        )) {
            if (
                [
                    "node_modules",
                    ".git",
                    ".mochi",
                    ".output",
                    ".tanstack",
                    ".cache",
                    ".DS_Store",
                    "target",
                    "dist",
                ].includes(e.name)
            )
                continue;
            if (e.isSymbolicLink()) throw Error("Symlink in benchmark source");
            const path = join(dir, e.name),
                key = prefix + e.name;
            if (e.isDirectory()) await walk(path, key + "/");
            else if (e.isFile()) {
                const file = Bun.file(path);
                bytes += file.size;
                if (
                    file.size > 128 * 1024 * 1024 ||
                    bytes > 2 * 1024 * 1024 * 1024 ||
                    Object.keys(entries).length >= 50000
                )
                    throw Error("Fingerprint budget exceeded");
                entries[key] = hash(await readFile(path));
            }
        }
    }
    await walk(resolve(root));
    if (!Object.keys(entries).length)
        throw Error("Empty fingerprint inventory");
    return {
        sha256: hash(JSON.stringify(entries)),
        files: Object.keys(entries).length,
        bytes,
        inventory: entries,
    };
}
