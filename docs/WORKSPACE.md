# The `.mochi/` workspace

`.mochi/` is a Git-ignored local workspace (also excluded from crate packages). It holds
two kinds of data that must never be confused:

- **Evidence and hand-authored work** – cannot be rebuilt. Never delete it automatically.
- **Regenerable output** – can be rebuilt from the evidence, the lockfile or the Rust source.

The layout below is what a populated workspace looked like when this was written; only the
rules in the two tables are guaranteed.

## Evidence (never auto-deleted)

| Path | What it is |
| --- | --- |
| `trials/*/**/capture*/` (a `manifest.json` beside a `site/` directory) | Original captures of live sites. Cannot be re-captured identically. |
| `source-packs/`, `semantic-conversion/` | Reviewed source specifications, reference measurements, benchmark results. |
| `previous-projects/` | Frozen copies of earlier projects, including recovered original controllers. |
| `<project>/.mochi/provenance/` | Recovered originals kept next to a project. |
| `<dir>/reconstruction/` | Derived models written by `check-evidence` / `model-project`. |
| `.git` anywhere | Repository data (and the evidence branch refs kept in the repository). |
| `projects/*` (the source files) | The canonical, **hand-edited** TanStack projects. `mochi clean` never touches their sources, but it does treat their `.output`, `.tanstack`, `.mochi/dist` (and, with `--deps`, `node_modules`) as regenerable like any other project's. They cannot be regenerated from a capture without losing edits, so protect them with `--keep` if their build output matters. |
| `handoff/`, `references/`, `qualification/`, `verification*/` | Handoff notes, reference observations, owned qualification fixtures and verification output. Skipped like the rows above. |

## Regenerable

| Artifact | Regenerate with |
| --- | --- |
| `<project>/.output`, `<project>/.tanstack`, `<project>/.mochi/dist`, `<project>/node_modules/.vite`, `.nitro`, `.vite-temp` | `bun install --frozen-lockfile && bun run build` in the project |
| `<project>/node_modules` | `bun install --frozen-lockfile` |
| `mochi-exports/*`, other generated projects | `mochi export <capture-or-site> --out <new-dir> ...` (see [EXPORT.md](EXPORT.md); `--out`/`-o` must not exist). Output depends on the generator version, so regenerating does not reproduce older, newer or hand-edited trees byte for byte. |
| `target/` (Cargo build cache, set by `.cargo/config.toml`) | `cargo build` / `cargo test` |

`.mochi/dist` is the build directory only for projects whose Vite config sets
`build.outDir: '.mochi/dist'` (the exported template does). Other `.mochi/*` files of a
project (`export.json`, `authored-*.json`, `style-audit.json`) are reports, not build output;
`mochi clean` leaves them alone.

## `mochi clean`

```sh
mochi clean [WORKSPACE]            # dry run (default WORKSPACE is .mochi): prints a table, deletes nothing
mochi clean --apply [WORKSPACE]    # deletes the rows marked "remove"
```

A *project root* is a directory containing `package.json` and `bun.lock`. For each one the
command lists (path, class, size, status):

| Class | Exact paths below the project root | Selected by |
| --- | --- | --- |
| `build` | `.output`, `.tanstack`, `.mochi/dist`, `node_modules/.vite`, `node_modules/.nitro`, `node_modules/.vite-temp` | default |
| `deps` | `node_modules` | `--deps` |
| `cargo` | a directory named `target`, not itself a project root, holding cargo's `CACHEDIR.TAG` (standard signature line, written by cargo) or a `.rustc_info.json` with a `rustc_fingerprint` | `--cargo` |

Unselected classes are still listed with the status `skipped`. Sizes are apparent sizes: the
sum of `st_size` over every entry, directories and symlinks included, with a hard-linked file
counted once per link; they are an estimate, not the space freed. On APFS, `bun install`
clones files from its cache, so deleting `node_modules` frees little real disk space. (One
observation on the development machine, not reproducible from the repository: deleting
621 MB of `node_modules` freed approximately no disk space.) The Cargo target and the bun
cache are where real space goes.

Keeping what you need:

- `--keep <PATH>` (repeatable) protects that project root, and any project below it, in every
  class; a path inside a project (for example `site/.output`) protects that whole project. The
  path must exist and must lie in the workspace (or contain it): otherwise it aborts before
  anything is planned, so a typo never silently protects nothing. A relative path is tried
  against the current directory and the workspace; the one inside the workspace wins. A path
  that goes through a symlink (other than one that leads to a directory containing the
  workspace, like `/tmp` on macOS) is refused: pass the real directory.
- `--keep-latest` keeps whole, for each *site family*, only the project whose newest build
  output is most recent, and removes the build outputs of older projects of that family. A
  project without any build output takes no part in the comparison (so it is not protected
  either: `--keep` your source projects, especially with `--deps`).
  A family is the `input_sha256` in the project's `.mochi/export.json`: every export of the same
  saved capture model shares it, whatever the directory is called. A project without one
  (for example a hand-edited `projects/*` tree) is its own family and is never superseded;
  directory names are deliberately not used to guess that two projects are the same site.
  "Newest" is the most recent modification time among the project's existing build outputs,
  including `node_modules/.vite|.nitro|.vite-temp`, whether or not `--deps` is given.

### Safety rules (enforced in `src/clean.rs`, covered by `tests/clean.rs`)

- Dry run is the default; `--apply` prints each path before deleting it and a final total.
  A second `--apply` finds nothing and succeeds. `--apply` exits non-zero if any removal
  failed or any row was `REFUSED` (after removing everything that could be removed).
  Control characters in names are printed as `\u{..}`.
- Only the exact directory names above are removed, at exactly those positions below a project
  root, compared against the on-disk spelling (on a case-insensitive filesystem `.OUTPUT` is not
  `.output`). No globs; look-alikes (`.output.bak`, `src/.output`, `dist`) are untouched. A project
  nested in `node_modules`, or any `.output`/`.tanstack`/`.mochi/dist`, is never scanned.
- Nothing outside the canonical workspace root is removed. A workspace whose final component
  is a symlink is refused however it is spelled (`link`, `link/`, `link/.`, `link/..`). A
  symlink in a *parent* component is resolved: `mochi clean lnk/p` with `lnk` a symlink cleans
  the real directory `p`, which is printed on the first line (`workspace: ...`). The filesystem
  root, `$HOME` (or, when `$HOME` is unset, the home directory from the user database; if that
  is unknown the command refuses to run) and any directory containing it are refused; a
  workspace that is, or lies inside, an evidence directory (by name, or a capture by its
  layout) is refused. A workspace containing git repositories gets a `WARNING` note: it is
  probably a source tree, not a `.mochi` workspace.
- Symlinks are never followed or deleted through: a symlinked candidate, or a candidate reached
  through a symlinked parent (for example a linked `.mochi` or `node_modules`), is refused and
  shown as such; symlinked directories are not scanned. Links *inside* a removed directory are
  unlinked, not followed. A candidate on another filesystem than its project, or containing a
  mount point, is refused.
- Deletion is relative to held directory descriptors, not paths (macOS and Linux; other
  systems refuse to run). The workspace root is opened once and must still be the directory
  that was planned (device and inode); each component down to the candidate is then opened
  relative to the previous descriptor without following symlinks and must be the planned
  directory; contents are removed with `unlinkat` relative to descriptors, re-checking each
  subdirectory for evidence and mount points, and the result is verified before it counts
  as removed. Swapping a parent directory for a symlink while `--apply` runs therefore makes
  the affected rows fail (they are reported) instead of deleting elsewhere. This is covered by
  a test that races `renameat(RENAME_SWAP)` against `--apply`. It does not defend against a
  process that is *allowed* to rename the planned directories themselves (the held descriptor
  follows the directory, not its name), or against someone replacing the workspace's own
  ancestors before the root is opened (the check is against the identity recorded at planning).
  Evidence is re-checked while descending, but a process that moves a capture's files *into* a
  build directory at the instant it is being emptied can still lose them (a manifest.json
  beside a site/ is a two-entry layout that cannot be checked atomically); nothing outside the
  workspace is touched.
- Evidence is never scanned or removed. Evidence directories are those named (ASCII
  case-insensitively) `provenance`, `source-packs`, `semantic-conversion`, `reconstruction`,
  `previous-projects`, `.git`, `handoff`, `references`, `qualification`, or starting with
  `capture` or `verification`, plus any directory with a `manifest.json` beside a `site/`
  directory (either may be a symlink; the match is deliberately broad). A candidate that
  itself is, or contains, one is refused. (Inside `node_modules` only the core names — not `.git`, `capture*`, `handoff`, `references`, `qualification` or `verification*` — and the capture layout count,
  because packages legitimately ship `.git` and `capture*` directories; inside cargo targets
  `.git` counts too, but `capture*` does not, because crates named `capture*` produce such
  build directories.)
- Build/deps candidates must sit under a project root. A cargo `target` needs its marker file
  and is refused while a build holds a `.cargo-lock` in it (checked with `flock`).
- An entry that cannot be read is reported and blocks removal of the directory containing it
  rather than being skipped.
- Directories named `.mochi-export-*` (exports in progress) are not scanned.

The command removes nothing it cannot list first. It does not stop running dev servers or
builds: do not clean a project that another process is using (a cargo target in use is
detected, a dev server's `.output` is not).

## Cargo build cache

`.mochi/target` is the largest regenerable item. The `[profile.dev]` settings in `Cargo.toml`
(see the comment there for the measured effect) keep it smaller. To reclaim space:

```sh
cargo clean -p mochi-web      # only this crate's artifacts; dependencies stay cached
mochi clean --cargo           # dry run, then add --apply, for the whole target directory
```

The next `cargo build` / `cargo test` rebuilds what was removed. Do not clean the target while
another build or test run is using it.

## Ports

Run one project per port at a time; a second process on the same port fails or silently
serves the wrong project.

- `mochi serve <site>` defaults to port **8000** (`--port` to change).
- A TanStack Start project exported with the current template (`src/export_template/vite.config.ts`)
  pins `server: { host: '127.0.0.1', port: 3000, strictPort: true }`, and its `dev` script
  passes no `--port`, so `bun run dev` listens on 127.0.0.1:3000 and fails on a conflict
  instead of moving to another port. `bun start` runs `bun .output/server/index.mjs`, whose
  Nitro `bun` preset reads `NITRO_PORT`, then `PORT`, and otherwise defaults to 3000 (nitro
  `runtime/internal/serve.mjs`, srvx 1.0.8 `resolvePortAndHost`). Projects exported earlier, or
  edited by hand such as `.mochi/projects/natural` (its Vite config sets only the host), do not
  pin the port, so `bun run dev` falls back to Vite's default 5173 (`DEFAULT_DEV_PORT`, Vite
  8.3.6) and moves on if it is busy. All of this was read from configuration and installed
  packages; I could not start a server here (the sandbox blocks local port binding), so the
  listening ports are not observed. To run two projects at once give one a different port
  (`PORT=4184 bun start`; for dev, Vite's `--port`).
- The long-running local previews used for the three example projects were Natural 4184,
  Infinite Garden 4185, Microsoft AI 4186 (recorded in `.mochi/semantic-conversion/servers.json`).
