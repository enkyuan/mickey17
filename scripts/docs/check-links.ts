#!/usr/bin/env bun
// Documentation reference checker: `bun scripts/docs/check-links.ts` (no dependencies).
//
// Fails (exit 1, one `file:line: message` per problem) when, in any text file tracked by git
// (or untracked and not ignored):
//   1. a relative markdown link `[t](path)` or reference definition `[t]: path` does not resolve
//      to a file or directory, or its `#fragment` is not a heading of the target .md file
//      (GitHub slug rules; duplicates get -1, -2, ...). Links inside fenced code blocks and
//      inline code are skipped; `http(s):`, `mailto:` and other schemes are not checked.
//   2. a mention `docs/<path>.md[#fragment]` anywhere (prose, code comments, strings, tests,
//      scripts, workflows) does not exist, or its fragment is not a heading.
//   3. a retired doc name listed in scripts/docs/renamed.txt (`old -> new`, one per line) is
//      still mentioned, as a full path, as a path below docs/, or as a bare `NAME.md`.
//      A line containing `check-links: allow` is exempt.
//   4. a file below docs/ breaks the naming convention in docs/README.md, or is not linked from
//      the docs/README.md index.
//
// Known limits (false negatives): HTML `<a href>`, setext (underlined) headings, links built
// at run time, bare mentions of a doc name that was never renamed, and absolute URLs are not
// checked; a fragment is checked only against ATX (`#`) headings.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { posix } from "node:path";

const root = execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
process.chdir(root);

/** Docs whose rename is deferred (open PRs name them by path); see docs/README.md. */
const PENDING_RENAME = ["docs/AGENT_SURFACE.md", "docs/DISTRIBUTION.md"];
const PENDING_RENAME_DIRS = ["docs/agent-surface/"];
const TEXT_EXT = /\.(md|rs|ts|tsx|js|mjs|json|toml|yml|yaml|sh|py|txt|css)$/;
const SKIP = /(^|\/)(Cargo\.lock|bun\.lock|package-lock\.json)$/;

const listed = execFileSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard"], {
  encoding: "utf8",
})
  .split("\0")
  .filter((f) => f && existsSync(f) && statSync(f).isFile());
const files = [...new Set(listed)].sort();
const textFiles = files.filter((f) => TEXT_EXT.test(f) && !SKIP.test(f));

const problems = new Set<string>();
const fail = (file: string, line: number, message: string) => problems.add(`${file}:${line}: ${message}`);
const read = (f: string) => readFileSync(f, "utf8");

/** GitHub-style heading slugs of an .md file (cached). */
const slugCache = new Map<string, Set<string>>();
function slugsOf(file: string): Set<string> {
  const hit = slugCache.get(file);
  if (hit) return hit;
  const slugs = new Set<string>();
  const seen = new Map<string, number>();
  let fence: string | null = null;
  for (const line of read(file).split("\n")) {
    const f = line.match(/^\s{0,3}(`{3,}|~{3,})/);
    if (f) {
      if (fence === null) fence = f[1][0];
      else if (f[1][0] === fence) fence = null;
      continue;
    }
    if (fence) continue;
    const h = line.match(/^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$/);
    if (!h) continue;
    const slug = h[1]
      .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
      .replace(/<[^>]+>/g, "")
      .replace(/`/g, "")
      .toLowerCase()
      .replace(/[^\p{L}\p{N}\s_-]/gu, "")
      .trim()
      .replace(/\s/g, "-");
    const n = seen.get(slug) ?? 0;
    seen.set(slug, n + 1);
    slugs.add(n === 0 ? slug : `${slug}-${n}`);
  }
  slugCache.set(file, slugs);
  return slugs;
}

function checkTarget(file: string, line: number, targetPath: string, fragment: string | undefined) {
  if (!existsSync(targetPath)) {
    fail(file, line, `link target does not exist: ${targetPath}`);
    return;
  }
  if (fragment && targetPath.endsWith(".md") && statSync(targetPath).isFile()) {
    let wanted = fragment;
    try {
      wanted = decodeURIComponent(fragment);
    } catch {}
    if (!slugsOf(targetPath).has(wanted.toLowerCase())) {
      fail(file, line, `anchor #${fragment} is not a heading of ${targetPath}`);
    }
  }
}

// ---- 1. markdown links ---------------------------------------------------------------------
const INLINE = /!?\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)/g;
const REFDEF = /^\s{0,3}\[[^\]]+\]:\s+<?(\S+?)>?(?:\s+.*)?$/;
for (const file of textFiles.filter((f) => f.endsWith(".md"))) {
  let fence: string | null = null;
  const lines = read(file).split("\n");
  lines.forEach((raw, i) => {
    const f = raw.match(/^\s{0,3}(`{3,}|~{3,})/);
    if (f) {
      if (fence === null) fence = f[1][0];
      else if (f[1][0] === fence) fence = null;
      return;
    }
    if (fence || raw.includes("check-links: allow")) return;
    const text = raw.replace(/`[^`]*`/g, (m) => " ".repeat(m.length));
    const targets: string[] = [];
    for (const m of text.matchAll(INLINE)) targets.push(m[1]);
    const def = text.match(REFDEF);
    if (def) targets.push(def[1]);
    for (const target of targets) {
      if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(target)) continue;
      const hash = target.indexOf("#");
      const pathPart = hash === -1 ? target : target.slice(0, hash);
      const fragment = hash === -1 ? undefined : target.slice(hash + 1);
      let decoded = pathPart;
      try {
        decoded = decodeURIComponent(pathPart);
      } catch {}
      const resolved = pathPart === ""
        ? file
        : decoded.startsWith("/")
          ? posix.normalize(decoded.slice(1))
          : posix.normalize(posix.join(posix.dirname(file), decoded));
      checkTarget(file, i + 1, resolved, fragment);
    }
  });
}

// ---- 2. docs/<path>.md mentions in any text file ----------------------------------------------
const MENTION = /(?<![A-Za-z0-9_./-])(docs\/[A-Za-z0-9_./-]+?\.md)(?![A-Za-z0-9_])(?:#([A-Za-z0-9_%-]+))?/g;
const SELF = new Set(["scripts/docs/check-links.ts", "scripts/docs/renamed.txt"]);
for (const file of textFiles) {
  if (SELF.has(file)) continue;
  read(file)
    .split("\n")
    .forEach((line, i) => {
      if (line.includes("check-links: allow")) return;
      for (const m of line.matchAll(MENTION)) checkTarget(file, i + 1, m[1], m[2]);
    });
}

// ---- 3. retired names ------------------------------------------------------------------------
const RENAMED = "scripts/docs/renamed.txt";
const retired: { old: string; neu: string; regexes: RegExp[] }[] = [];
if (existsSync(RENAMED)) {
  for (const line of read(RENAMED).split("\n")) {
    const m = line.match(/^(\S+)\s+->\s+(\S+)/);
    if (!m) continue;
    const [, old, neu] = m;
    const forms = new Set<string>([old]);
    if (old.startsWith("docs/")) {
      const below = old.slice("docs/".length);
      if (below.includes("/")) forms.add(below);
    }
    if (old.endsWith(".md")) {
      const base = posix.basename(old);
      if (base !== posix.basename(neu)) forms.add(base);
    }
    const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    retired.push({
      old,
      neu,
      regexes: [...forms].map((f) => new RegExp(`(?<![A-Za-z0-9_./-])${esc(f)}(?![A-Za-z0-9_])`)),
    });
  }
}
for (const file of textFiles) {
  if (file === RENAMED) continue;
  read(file)
    .split("\n")
    .forEach((line, i) => {
      if (line.includes("check-links: allow")) return;
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
if (existsSync("docs/README.md")) {
  const index = read("docs/README.md");
  const linked = new Set<string>();
  for (const m of index.matchAll(INLINE)) {
    const p = m[1].split("#")[0];
    if (p && !/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(p)) linked.add(posix.normalize(posix.join("docs", p)));
  }
  for (const f of docFiles) {
    if (f !== "docs/README.md" && !linked.has(f)) fail(f, 1, "not linked from the docs/README.md index");
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
