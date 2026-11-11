#!/usr/bin/env bun
// Documentation reference checker: `bun scripts/docs/check-links.ts` (no dependencies).
//
// Fails (exit 1, one `file:line: message` per problem) when, in any non-binary file tracked by
// git (or untracked and not ignored):
//   1. a relative markdown link `[t](path)` (also `<path>` targets, links whose text spans lines,
//      and reference definitions `[t]: path`) does not resolve to a tracked file or directory, or
//      its `#fragment` is not a heading slug of the target .md file (GitHub rules, exact case,
//      duplicates get -1, -2, ...). Fenced code blocks and inline code are skipped; `http(s):`,
//      `mailto:` and other schemes are not checked.
//   2. a mention `docs/<path>.md[#fragment]` anywhere (prose, code comments, strings, tests,
//      scripts, workflows, with or without a `./`, `../` or other prefix) does not exist, or its
//      fragment is not a heading.
//   3. a retired doc name listed in scripts/docs/renamed.txt (`old -> new`, one per line) is still
//      mentioned: as a full path, as a path below docs/, or as a bare `NAME.md`; a retired
//      directory (`docs/parity-reports/`) is matched with or without the trailing slash.
//   4. a file below docs/ breaks the naming convention in docs/README.md, or is not linked from a
//      table row of the docs/README.md index.
// A line containing `check-links: allow` is exempt from 1-3.
//
// Existence is decided against the git file list with exact case, so `docs/Parity.md` fails on a
// case-insensitive filesystem too, and a git-ignored local file is not a valid target.
//
// Known limits (false negatives): HTML `<a href>`, setext (underlined) headings, links built at
// run time, bare mentions of a doc name that was never renamed, and absolute URLs are not checked;
// a link through a (tracked) symlinked directory is reported as missing; indented (4-space) code
// blocks without a fence are not masked.
import { execFileSync } from "node:child_process";
import { readFileSync, statSync } from "node:fs";
import { posix } from "node:path";

const root = execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
process.chdir(root);

/** Docs whose rename is deferred (open PRs name them by path); see docs/README.md. */
const PENDING_RENAME = ["docs/AGENT_SURFACE.md", "docs/DISTRIBUTION.md"];
const PENDING_RENAME_DIRS = ["docs/agent-surface/"];
const SKIP = /(^|\/)(Cargo\.lock|bun\.lock|bun\.lockb|package-lock\.json)$/;
const MAX_BYTES = 4 << 20;

const listed = execFileSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard"], {
  encoding: "utf8",
  maxBuffer: 256 << 20,
})
  .split("\0")
  .filter(Boolean);
const files: string[] = [];
for (const f of [...new Set(listed)].sort()) {
  try {
    if (statSync(f).isFile()) files.push(f);
  } catch {}
}
const fileSet = new Set(files);
const dirSet = new Set<string>();
for (const f of files) for (let d = posix.dirname(f); d !== "."; d = posix.dirname(d)) dirSet.add(d);
const exists = (p: string) => fileSet.has(p) || dirSet.has(p.replace(/\/$/, ""));

const contents = new Map<string, string>();
const textFiles: string[] = [];
for (const f of files) {
  if (SKIP.test(f) || statSync(f).size > MAX_BYTES) continue;
  const buf = readFileSync(f);
  if (buf.subarray(0, 8000).includes(0)) continue; // binary
  contents.set(f, buf.toString("utf8"));
  textFiles.push(f);
}
const read = (f: string) => contents.get(f) ?? readFileSync(f, "utf8");

const problems = new Set<string>();
const fail = (file: string, line: number, message: string) => problems.add(`${file}:${line}: ${message}`);
/** 1-based line of a character index (binary search over line starts). */
function lineStarts(text: string): number[] {
  const starts = [0];
  for (let i = text.indexOf("\n"); i !== -1; i = text.indexOf("\n", i + 1)) starts.push(i + 1);
  return starts;
}
function lineAt(starts: number[], index: number): number {
  let lo = 0;
  let hi = starts.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (starts[mid] <= index) lo = mid;
    else hi = mid - 1;
  }
  return lo + 1;
}

/** Blank out fenced code blocks, inline code and HTML comments, keeping every newline. */
function mask(text: string): string {
  const blank = (s: string) => s.replace(/[^\n]/g, " ");
  const out: string[] = [];
  let fence: string | null = null;
  for (const line of text.split("\n")) {
    const f = line.match(/^\s*(`{3,}|~{3,})/);
    if (f) {
      if (fence === null) fence = f[1][0];
      else if (f[1][0] === fence) fence = null;
      out.push(blank(line));
      continue;
    }
    out.push(fence ? blank(line) : line);
  }
  // Inline code may span a line break but not a blank line.
  return out
    .join("\n")
    .replace(/`[^`\n]*(?:\n[^`\n]+)*`/g, blank)
    .replace(/<!--[\s\S]*?-->/g, blank);
}

/** GitHub-style heading slugs of an .md file (cached). */
const slugCache = new Map<string, Set<string>>();
function slugsOf(file: string): Set<string> {
  const hit = slugCache.get(file);
  if (hit) return hit;
  const slugs = new Set<string>();
  const seen = new Map<string, number>();
  const masked = mask(read(file)).split("\n");
  const original = read(file).split("\n");
  masked.forEach((m, i) => {
    if (!/^\s{0,3}#{1,6}\s/.test(m)) return;
    const h = original[i].match(/^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$/);
    if (!h) return;
    const base = h[1]
      .replace(/!?\[([^\]]*)\](?:\([^)]*\)|\[[^\]]*\])/g, "$1")
      .replace(/<[^>]+>/g, "")
      .replace(/&amp;/g, "&")
      .replace(/&lt;/g, "<")
      .replace(/&gt;/g, ">")
      .replace(/&quot;/g, '"')
      .replace(/&#39;/g, "'")
      .replace(/`/g, "")
      .replace(/\*+/g, "")
      .replace(/(?<![A-Za-z0-9])_{1,2}([^_]+?)_{1,2}(?![A-Za-z0-9])/g, "$1")
      .toLowerCase()
      .replace(/[^\p{L}\p{M}\p{N} _-]/gu, "")
      .replace(/ /g, "-");
    // github-slugger: a repeated slug gets -1, -2, ... and the result is itself counted.
    let slug = base;
    while (seen.has(slug)) {
      const n = (seen.get(base) ?? 0) + 1;
      seen.set(base, n);
      slug = `${base}-${n}`;
    }
    seen.set(slug, seen.get(slug) ?? 0);
    slugs.add(slug);
  });
  slugCache.set(file, slugs);
  return slugs;
}

function checkTarget(file: string, line: number, targetPath: string, fragment: string | undefined) {
  if (!exists(targetPath)) {
    fail(file, line, `link target does not exist (exact case, tracked): ${targetPath}`);
    return;
  }
  if (fragment && targetPath.endsWith(".md") && fileSet.has(targetPath)) {
    let wanted = fragment;
    try {
      wanted = decodeURIComponent(fragment);
    } catch {}
    if (!slugsOf(targetPath).has(wanted)) fail(file, line, `anchor #${fragment} is not a heading slug of ${targetPath}`);
  }
}

// ---- 1. markdown links ---------------------------------------------------------------------
const INLINE = /(?<!\\)!?\[(?:[^\[\]]|\[[^\[\]]{0,200}\]){0,500}\]\(\s*(<[^>\n]+>|[^)\s]+)(?:\s+"[^"]*")?\s*\)/g;
const REFDEF = /^\s{0,3}\[(?!\^)[^\]]+\]:\s+(<[^>]+>|\S+)/;
const isExternal = (t: string) => /^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(t);
function resolveLink(file: string, target: string): { path: string; fragment?: string } | null {
  const t = target.replace(/^<|>$/g, "");
  if (isExternal(t)) return null;
  const hash = t.indexOf("#");
  let pathPart = hash === -1 ? t : t.slice(0, hash);
  const fragment = hash === -1 ? undefined : t.slice(hash + 1);
  pathPart = pathPart.replace(/\?.*$/, "");
  try {
    pathPart = decodeURIComponent(pathPart);
  } catch {}
  if (pathPart === "") return { path: file, fragment };
  const path = pathPart.startsWith("/")
    ? posix.normalize(pathPart.slice(1))
    : posix.normalize(posix.join(posix.dirname(file), pathPart));
  return { path: path.replace(/\/$/, ""), fragment };
}
/** Lines of `file` that carry the allow marker (1-based). */
const allowed = (file: string) => {
  const set = new Set<number>();
  read(file)
    .split("\n")
    .forEach((l, i) => l.includes("check-links: allow") && set.add(i + 1));
  return set;
};
for (const file of textFiles.filter((f) => f.endsWith(".md"))) {
  const text = read(file);
  const masked = mask(text);
  const ok = allowed(file);
  const starts = lineStarts(text);
  const found: { index: number; end: number; target: string }[] = [];
  for (const m of masked.matchAll(INLINE)) found.push({ index: m.index ?? 0, end: (m.index ?? 0) + m[0].length, target: m[1] });
  let offset = 0;
  for (const line of masked.split("\n")) {
    const d = line.match(REFDEF);
    if (d) found.push({ index: offset, end: offset + line.length, target: d[1] });
    offset += line.length + 1;
  }
  for (const { index, end, target } of found) {
    const line = lineAt(starts, index);
    // The allow marker may sit on any line of a multi-line link.
    if ([...ok].some((l) => l >= line && l <= lineAt(starts, end))) continue;
    const r = resolveLink(file, target);
    if (r) checkTarget(file, line, r.path, r.fragment);
  }
}

// ---- 2. docs/<path>.md mentions in any text file ----------------------------------------------
// `docs/x.md`, `./docs/x.md`, `../docs/x.md`; not `…/docs/x.md` of another tree or a URL.
const MENTION =
  /(?:(?<=(?:^|[^A-Za-z0-9_./-])(?:\.{1,2}\/)+)|(?<![A-Za-z0-9_./-]))(docs\/[A-Za-z0-9_./-]+?\.md)(?![A-Za-z0-9_])(?:#([\p{L}\p{N}_%-]+))?/gu;
const SELF = new Set(["scripts/docs/check-links.ts", "scripts/docs/renamed.txt"]);
for (const file of textFiles) {
  if (SELF.has(file)) continue;
  const ok = allowed(file);
  read(file)
    .split("\n")
    .forEach((line, i) => {
      if (ok.has(i + 1)) return;
      for (const m of line.matchAll(MENTION)) checkTarget(file, i + 1, m[1], m[2]);
    });
}

// ---- 3. retired names ------------------------------------------------------------------------
const RENAMED = "scripts/docs/renamed.txt";
const retired: { old: string; neu: string; regexes: RegExp[] }[] = [];
const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
if (fileSet.has(RENAMED)) {
  for (const line of read(RENAMED).split("\n")) {
    const m = line.match(/^(\S+)\s+->\s+(\S+)/);
    if (!m) continue;
    const [, old, neu] = m;
    const dir = old.endsWith("/");
    // Full path: also caught behind `/`, `./`, `../`. Shorter forms must not follow a `/` or `.`.
    const regexes = [
      new RegExp(`(?<![A-Za-z0-9_-])${esc(dir ? old.slice(0, -1) : old)}(?![A-Za-z0-9_${dir ? "-" : ""}])`),
    ];
    if (!dir) {
      const shorter = new Set<string>();
      if (old.startsWith("docs/") && old.slice("docs/".length).includes("/")) shorter.add(old.slice("docs/".length));
      const base = posix.basename(old);
      if (old.endsWith(".md") && base !== posix.basename(neu)) shorter.add(base);
      for (const f of shorter) regexes.push(new RegExp(`(?<![A-Za-z0-9_./-])${esc(f)}(?![A-Za-z0-9_])`));
    }
    retired.push({ old, neu, regexes });
  }
}
for (const file of textFiles) {
  if (SELF.has(file)) continue;
  const ok = allowed(file);
  read(file)
    .split("\n")
    .forEach((line, i) => {
      if (ok.has(i + 1)) return;
      for (const r of retired) {
        if (r.regexes.some((re) => re.test(line))) fail(file, i + 1, `retired doc name ${r.old} (now ${r.neu})`);
      }
    });
}

// ---- 4. naming convention and index -------------------------------------------------------------
const KEBAB = /^[a-z0-9]+(-[a-z0-9]+)*$/;
const docFiles = files.filter((f) => f.startsWith("docs/") && f.endsWith(".md"));
for (const f of docFiles) {
  if (PENDING_RENAME.includes(f) || PENDING_RENAME_DIRS.some((d) => f.startsWith(d))) continue;
  const parts = f.split("/").slice(1);
  const name = parts[parts.length - 1].replace(/\.md$/, "");
  for (const dir of parts.slice(0, -1)) if (!KEBAB.test(dir)) fail(f, 1, `directory name is not lowercase kebab-case: ${dir}`);
  if (f === "docs/README.md") continue;
  if (parts[0] === "releases" && parts.length === 2) {
    if (!/^(NEXT|\d+\.\d+\.\d+(-[A-Za-z0-9.]+)?)$/.test(name)) fail(f, 1, "release notes are NEXT.md or <version>.md");
  } else if (parts[0] === "reports") {
    if (!/^\d{4}-\d{2}-\d{2}-[a-z0-9]+(-[a-z0-9]+)*$/.test(name)) fail(f, 1, "reports are named YYYY-MM-DD-slug.md");
  } else if (!KEBAB.test(name)) {
    fail(f, 1, `doc name is not lowercase kebab-case: ${name}.md`);
  }
}
if (fileSet.has("docs/README.md")) {
  // Only links in table rows count as the index (not comments, code or free prose).
  const rows = mask(read("docs/README.md"))
    .split("\n")
    .filter((l) => l.trimStart().startsWith("|"))
    .join("\n");
  const linked = new Set<string>();
  for (const m of rows.matchAll(INLINE)) {
    const r = resolveLink("docs/README.md", m[1]);
    if (r) linked.add(r.path);
  }
  for (const f of docFiles) {
    if (f !== "docs/README.md" && !linked.has(f)) fail(f, 1, "not linked from a table row of the docs/README.md index");
  }
} else {
  fail("docs/README.md", 1, "missing");
}

if (problems.size > 0) {
  console.error([...problems].sort().join("\n"));
  console.error(`\n${problems.size} documentation reference problem(s).`);
  process.exit(1);
}
console.log(`docs references ok: ${textFiles.length} text files, ${docFiles.length} docs checked.`);
