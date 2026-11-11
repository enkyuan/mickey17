#!/usr/bin/env bash
# Remove an explicit allowlist of regenerable or historical-generated items from a `.mochi/`
# workspace that has been re-homed by scripts/workspace/migrate.sh (NEW layout paths).
#
# Dry run by default. `--apply` removes. Only the exact workspace-relative paths listed in this
# file can ever be removed; anything else is refused, never skipped silently. Original captures,
# non-canonical capture variants, source packs, previous-projects, semantic-conversion, references,
# qualification, handoff, verification output and hand-edited `projects/` are NOT on the list and
# are additionally protected by the evidence checks below.
#
# Groups (run in this order; `--only a,b` selects some):
#   empties         cli-release, cli-final (empty directories): `rmdir`, fails if not empty
#   gates           reference.json + candidate.json (6 files) inside 3 parity gate directories: `rm -f`.
#                   The gate's check-parity-report.json, contract.json and log stay: the committed
#                   docs/reports/2026-11-10-parity-first-scoped.md says the reports are retained.
#   dup-exports     3 duplicate exports (same input_sha256 as a kept export): `rm -r`
#   trial-projects  29 generated project directories inside evidence/trials: `rm -r`
#   node-modules    node_modules of the exports: `mochi clean --deps --apply` (descriptor based)
#   cargo-target    cache/target: `mochi clean --cargo --apply` (refuses while a build holds .cargo-lock)
#
# Refuses (exit 2, nothing removed) when: a path is not on the allowlist, is a symlink, is not a
# directory on the workspace's filesystem, contains a symlink (outside node_modules), has an
# evidence-named entry or a manifest.json beside a site/ inside, is held open by a process
# (lsof), a cargo/rustc process runs, `.cargo-lock` is held, or something listens on 3000, 8000,
# 4184-4186. A missing allowlisted path is reported as already gone.
#
# `node-modules` and `cargo-target` also refuse while any other deps/build output exists in the
# workspace (a `.output`, `.tanstack`, `.mochi/dist` or other `node_modules` would be listed by
# `mochi clean` and is not on the allowlist): run `mochi clean` by hand for those first.
#
# Limit: the `rm` groups work on paths, not directory descriptors (unlike `mochi clean`); the
# checks run first and the allowlist is fixed, but a process that swaps a directory for a symlink
# between the check and the `rm` is not defended against. Run it with nothing else active.
#
# Usage: scripts/workspace/prune.sh [--workspace DIR] [--mochi BINARY] [--only g1,g2] [--dry-run | --apply]
set -euo pipefail

APPLY=0
WORKSPACE=""
MOCHI=""
ONLY="empties,gates,dup-exports,trial-projects,node-modules,cargo-target"
while [ $# -gt 0 ]; do
  case "$1" in
    --apply) APPLY=1 ;;
    --dry-run) APPLY=0 ;;
    --workspace) shift; WORKSPACE="${1:?--workspace needs a directory}" ;;
    --mochi) shift; MOCHI="${1:?--mochi needs a binary}" ;;
    --only) shift; ONLY="${1:?--only needs a comma separated list}" ;;
    -h|--help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 64 ;;
  esac
  shift
done
if [ -z "$WORKSPACE" ]; then
  WORKSPACE="$(cd "$(dirname "$0")/../.." && pwd)/.mochi"
fi
[ -d "$WORKSPACE" ] || { echo "refusing: workspace not found: $WORKSPACE" >&2; exit 2; }
[ ! -L "$WORKSPACE" ] || { echo "refusing: workspace is a symlink: $WORKSPACE" >&2; exit 2; }
M="$(cd "$WORKSPACE" && pwd -P)"
[ -n "$M" ] && [ "$M" != "/" ] || { echo "refusing: bad workspace" >&2; exit 2; }

# ---- the allowlist: exact paths relative to the workspace --------------------------------------
EMPTIES=(cli-release cli-final)
GATES=(
  generated/parity-runs/natural-compat-core-1-gate/reference.json
  generated/parity-runs/natural-compat-core-1-gate/candidate.json
  generated/parity-runs/garden-hand-core-1-gate/reference.json
  generated/parity-runs/garden-hand-core-1-gate/candidate.json
  generated/parity-runs/garden-recover-core-1-gate/reference.json
  generated/parity-runs/garden-recover-core-1-gate/candidate.json
)
DUP_EXPORTS=(
  generated/exports/rec3-natural
  generated/exports/rec3-infinitegarden
  generated/exports/rr-natural2
)
# Generated TanStack projects (no node_modules, no captures, no clone-log entries) inside the
# trials. NOT included, because tracked docs cite them: 20261108-microsoft-ai/microsoft-ai
# (VALIDATION.md, docs/real-world-trials.md) and 20261108-client-recovery/{natural-final,
# infinitegarden-final} ("final projects", same two files).
TRIAL_PROJECTS=(
  "evidence/trials/20261108-client-recovery/natural"
  "evidence/trials/20261108-client-recovery/natural-current"
  "evidence/trials/20261108-client-recovery/natural-ready"
  "evidence/trials/20261108-client-recovery/natural-structure-bun1311"
  "evidence/trials/20261108-client-recovery/project"
  "evidence/trials/20261108-client-recovery/project-current"
  "evidence/trials/20261108-client-recovery/project-final"
  "evidence/trials/20261108-microsoft-ai/project-complete"
  "evidence/trials/20261108-microsoft-ai/project-final"
  "evidence/trials/20261108-microsoft-ai/project-ready"
  "evidence/trials/20261108-microsoft-ai/project-verified"
  "evidence/trials/20261108-normalized/infinitegarden/project"
  "evidence/trials/20261108-normalized/natural/project"
  "evidence/trials/20261108-reconstruction/infinitegarden/project"
  "evidence/trials/20261108-reconstruction/infinitegarden/project-background-legacy"
  "evidence/trials/20261108-reconstruction/infinitegarden/project-before-animation-state"
  "evidence/trials/20261108-reconstruction/natural/project"
  "evidence/trials/20261108-reconstruction/natural/project-background-legacy"
  "evidence/trials/20261108-reconstruction/natural/project-before-animation-state"
  "evidence/trials/20261108-renderer-auto/infinitegarden/project"
  "evidence/trials/20261108-surfaces-final/infinitegarden/project"
  "evidence/trials/20261108-surfaces-final/natural/project"
  "evidence/trials/20261108-surfaces-v1/infinitegarden/project"
  "evidence/trials/20261108-surfaces-v2/natural/project"
  "evidence/trials/20261108-surfaces-v2/natural/project-verified"
  "evidence/trials/20261108-surfaces-v2/natural/project-visible"
  "evidence/trials/20261108-surfaces-v4/infinitegarden/project"
  "evidence/trials/20261108-synchronized/infinitegarden/project"
  "evidence/trials/20261108-synchronized/natural/project"
)
NODE_MODULES=(
  generated/exports/infinitegarden-compat/node_modules
  generated/exports/natural-compat/node_modules
  generated/exports/rec3-infinitegarden/node_modules
  generated/exports/rec3-natural/node_modules
  generated/exports/rr-natural2/node_modules
)
CARGO_TARGET=(cache/target)

fail=0
problem() { echo "refusing: $*" >&2; fail=1; }
device() { stat -f %d "$1" 2>/dev/null || stat -c %d "$1"; }
selected() { case ",$ONLY," in *",$1,"*) return 0 ;; *) return 1 ;; esac; }
for g in $(echo "$ONLY" | tr ',' ' '); do
  case "$g" in
    empties|gates|dup-exports|trial-projects|node-modules|cargo-target) ;;
    *) echo "unknown group: $g" >&2; exit 64 ;;
  esac
done

# True when $1 is on the allowlist (exact string match).
allowed() {
  local p="$1" a
  for a in "${EMPTIES[@]}" "${GATES[@]}" "${DUP_EXPORTS[@]}" "${TRIAL_PROJECTS[@]}" "${NODE_MODULES[@]}" "${CARGO_TARGET[@]}"; do
    if [ "$a" = "$p" ]; then return 0; fi
  done
  return 1
}

# True when no component of the workspace-relative path $1 (below the workspace) is a symlink.
chain_ok() {
  local cur="$M" part
  local -a parts
  IFS=/ read -r -a parts <<< "$1"
  for part in "${parts[@]}"; do
    cur="$cur/$part"
    if [ -L "$cur" ]; then return 1; fi
  done
  return 0
}

WS_DEV="$(device "$M")"
TOTAL_KB=0
COUNT=0

# Checks one allowlisted regular file (a `gates` entry): not a symlink, in a real directory on
# the workspace's filesystem, not held open.
check_file() {
  local rel="$1" p="$M/$1" kb
  if ! allowed "$rel"; then problem "not on the allowlist: $rel"; return 0; fi
  case "$rel" in /*|*..*|*//*|"") problem "malformed allowlist path: $rel"; return 0 ;; esac
  if ! chain_ok "$rel"; then problem "symlink in the path: $rel"; return 0; fi
  if [ ! -e "$p" ]; then echo "already gone: $rel"; return 0; fi
  if [ ! -f "$p" ]; then problem "not a regular file: $rel"; return 0; fi
  if [ "$(device "$p")" != "$WS_DEV" ]; then problem "on another filesystem: $rel"; return 0; fi
  if [ -n "$(lsof -nP -t "$p" 2>/dev/null | head -1)" ]; then problem "held open by a process: $rel"; return 0; fi
  kb="$(du -sk "$p" | cut -f1)"
  printf '%-90s %10s KiB\n' "$rel" "$kb"
  TOTAL_KB=$((TOTAL_KB + kb)); COUNT=$((COUNT + 1))
}

# Checks one allowlisted path. `kind`: empty (rmdir), full (rm -r) or nodemodules (mochi clean).
check_path() {
  local rel="$1" kind="$2" p="$M/$1" kb devs link ev m
  if ! allowed "$rel"; then problem "not on the allowlist: $rel"; return 0; fi
  case "$rel" in /*|*..*|*//*|"") problem "malformed allowlist path: $rel"; return 0 ;; esac
  if ! chain_ok "$rel"; then problem "symlink in the path: $rel"; return 0; fi
  if [ ! -e "$p" ]; then echo "already gone: $rel"; return 0; fi
  if [ ! -d "$p" ]; then problem "not a directory: $rel"; return 0; fi
  if [ "$(device "$p")" != "$WS_DEV" ]; then problem "on another filesystem: $rel"; return 0; fi
  kb="$(du -sk "$p" | cut -f1)"
  if [ "$kind" = empty ]; then
    if [ -n "$(find "$p" -mindepth 1 -print -quit)" ]; then problem "not empty: $rel"; return 0; fi
  else
    # No mount point inside, no symlink outside node_modules (rm never follows, but refuse anyway).
    devs="$(find "$p" -type d -exec sh -c 'for d; do stat -f %d "$d" 2>/dev/null || stat -c %d "$d"; done' _ {} + | sort -u | wc -l | tr -d ' ')"
    if [ "$devs" != 1 ]; then problem "holds a mount point: $rel"; return 0; fi
    if [ "$kind" != nodemodules ]; then
      link="$(find "$p" -path '*/node_modules' -prune -o -type l -print -quit)"
      if [ -n "$link" ]; then problem "contains a symlink ($link): $rel"; return 0; fi
    fi
    # Evidence rules (docs/workspace.md): evidence-named entries or a capture layout inside.
    ev="$(find "$p" -path '*/node_modules' -prune -o \( -iname provenance -o -iname source-packs -o -iname semantic-conversion -o -iname 'capture*' -o -iname reconstruction -o -iname previous-projects -o -name .git -o -iname handoff -o -iname references -o -iname qualification -o -iname 'verification*' -o -iname evidence \) -print -quit)"
    if [ -n "$ev" ]; then problem "contains evidence ($ev): $rel"; return 0; fi
    if [ "$kind" != nodemodules ]; then
      while IFS= read -r m; do
        if [ -d "$(dirname "$m")/site" ]; then problem "contains a capture layout ($m): $rel"; return 0; fi
      done < <(find "$p" -path '*/node_modules' -prune -o -name manifest.json -print)
    fi
    case "$(basename "$p")" in
      provenance|source-packs|semantic-conversion|capture*|reconstruction|previous-projects|.git|handoff|references|qualification|verification*|evidence)
        problem "evidence name: $rel"; return 0 ;;
    esac
  fi
  if command -v lsof >/dev/null 2>&1; then
    if [ -n "$(lsof -nP -t +D "$p" 2>/dev/null | head -1)" ]; then problem "held open by a process: $rel"; return 0; fi
  else
    problem "lsof not found, cannot check open files: $rel"; return 0
  fi
  printf '%-90s %10s KiB\n' "$rel" "$kb"
  TOTAL_KB=$((TOTAL_KB + kb)); COUNT=$((COUNT + 1))
}

# ---- preflight --------------------------------------------------------------------------------
# lsof exits 1 both for "nothing found" and for some errors, so check that it works at all, and
# also try to connect.
busy=""
if ! command -v lsof >/dev/null 2>&1 || ! lsof -nP -p $$ >/dev/null 2>&1; then
  problem "lsof is missing or not working, cannot check ports or open files"
fi
for port in 3000 8000 4184 4185 4186; do
  if { command -v nc >/dev/null 2>&1 && nc -z 127.0.0.1 "$port" >/dev/null 2>&1; } \
    || [ -n "$(lsof -nP -iTCP:"$port" -sTCP:LISTEN -t 2>/dev/null || true)" ]; then
    busy="$busy $port"
  fi
done
if [ -n "$busy" ]; then problem "something is listening on port(s):$busy"; fi
if pgrep -x cargo >/dev/null 2>&1 || pgrep -x rustc >/dev/null 2>&1; then
  problem "a cargo or rustc process is running"
fi
if selected cargo-target && [ -d "$M/cache/target" ]; then
  while IFS= read -r lock; do
    if [ -n "$(lsof -nP -t "$lock" 2>/dev/null | head -1)" ]; then problem ".cargo-lock is held: ${lock#"$M"/}"; fi
  done < <(find "$M/cache/target" -maxdepth 3 -name .cargo-lock)
fi

echo "workspace: $M"
if selected empties; then for r in "${EMPTIES[@]}"; do check_path "$r" empty; done; fi
if selected gates; then for r in "${GATES[@]}"; do check_file "$r"; done; fi
if selected dup-exports; then for r in "${DUP_EXPORTS[@]}"; do check_path "$r" full; done; fi
if selected trial-projects; then for r in "${TRIAL_PROJECTS[@]}"; do check_path "$r" full; done; fi
if selected node-modules; then for r in "${NODE_MODULES[@]}"; do check_path "$r" nodemodules; done; fi
if selected cargo-target; then for r in "${CARGO_TARGET[@]}"; do check_path "$r" nodemodules; done; fi

# ---- mochi clean for the build-output groups ----------------------------------------------------
TMPBIN=""
cleanup() { if [ -n "$TMPBIN" ]; then rm -f -- "$TMPBIN"; fi; }
trap cleanup EXIT
# `mochi` is only needed when there is something left for it to remove.
need_nm=0
need_cargo=0
if selected node-modules; then
  for r in "${NODE_MODULES[@]}"; do if [ -d "$M/$r" ]; then need_nm=1; fi; done
fi
if selected cargo-target && [ -d "$M/cache/target" ]; then need_cargo=1; fi
need_mochi=$((need_nm + need_cargo))
if [ "$need_mochi" -ge 1 ]; then
  if [ -z "$MOCHI" ]; then
    for c in "$M/cache/target/release/mochi" "$M/cache/target/debug/mochi" "$(command -v mochi || true)"; do
      if [ -n "$c" ] && [ -x "$c" ] && "$c" clean --help >/dev/null 2>&1; then MOCHI="$c"; break; fi
    done
  fi
  if [ -z "$MOCHI" ] || ! "$MOCHI" clean --help >/dev/null 2>&1; then
    problem "no usable mochi binary (with the clean subcommand) found; pass --mochi BINARY"
  fi
fi
if [ "$fail" -eq 0 ] && [ "$need_mochi" -ge 1 ] && [ -n "$MOCHI" ]; then
  # The binary may live in the target that is about to be removed: use a private copy.
  TMPBIN="$(mktemp "${TMPDIR:-/tmp}/mochi-prune.XXXXXX")"
  cp "$MOCHI" "$TMPBIN"
  chmod +x "$TMPBIN"
  # Every `remove` row of the dry run must be on the allowlist.
  plan_check() {
    local flag="$1" out row path
    if ! out="$("$TMPBIN" clean "$flag" "$M")"; then
      problem "mochi clean $flag failed"; return 0
    fi
    while IFS= read -r row; do
      path="${row%%  *}"
      if ! allowed "$path"; then problem "mochi clean $flag would remove a path not on the allowlist: $path"; fi
    done < <(printf '%s\n' "$out" | awk '/  remove$/')
  }
  if [ "$need_nm" -eq 1 ]; then plan_check --deps; fi
  if [ "$need_cargo" -eq 1 ]; then plan_check --cargo; fi
fi

if [ "$fail" -ne 0 ]; then
  echo "nothing was removed." >&2
  exit 2
fi
echo "$COUNT path(s), $((TOTAL_KB / 1024)) MiB allocated (APFS clones share blocks; real space freed is less)."
if [ "$APPLY" -ne 1 ]; then
  echo "dry run: nothing removed. Re-run with --apply."
  exit 0
fi

# ---- apply --------------------------------------------------------------------------------------
if selected empties; then
  for r in "${EMPTIES[@]}"; do
    if [ -d "${M:?}/${r:?}" ]; then rmdir -- "${M:?}/${r:?}"; echo "removed $r"; fi
  done
fi
remove_all() {
  local r
  for r in "$@"; do
    if [ -d "${M:?}/${r:?}" ]; then
      if ! allowed "$r"; then echo "internal error: $r not allowlisted" >&2; exit 3; fi
      if ! chain_ok "$r"; then echo "error: symlink appeared in $r" >&2; exit 3; fi
      echo "removing $r"
      rm -rf -- "${M:?}/${r:?}"
      echo "removed $r"
    fi
  done
}
if selected gates; then
  for r in "${GATES[@]}"; do
    if [ -f "${M:?}/${r:?}" ] && [ ! -L "${M:?}/${r:?}" ]; then
      if ! allowed "$r"; then echo "internal error: $r not allowlisted" >&2; exit 3; fi
      if ! chain_ok "$r"; then echo "error: symlink appeared in $r" >&2; exit 3; fi
      rm -f -- "${M:?}/${r:?}"
      echo "removed $r"
    fi
  done
fi
if selected dup-exports; then remove_all "${DUP_EXPORTS[@]}"; fi
if selected trial-projects; then remove_all "${TRIAL_PROJECTS[@]}"; fi
# Only what the preflight cross-checked against the allowlist.
if [ "$need_nm" -eq 1 ] && [ -n "$TMPBIN" ]; then "$TMPBIN" clean --deps --apply "$M"; fi
if [ "$need_cargo" -eq 1 ] && [ -n "$TMPBIN" ]; then "$TMPBIN" clean --cargo --apply "$M"; fi
echo "done."
