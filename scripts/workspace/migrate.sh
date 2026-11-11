#!/usr/bin/env bash
# Re-home the top level of a `.mochi/` workspace into the layout of docs/WORKSPACE.md:
#
#   evidence/   generated/   runs/   cache/   projects/ (unchanged)
#
# Dry run by default: prints every `old -> new` move and changes nothing. `--apply` performs
# the moves with `mv` on a single filesystem (a rename, never a copy) and appends a reversal map
# to `.mochi/handoff/workspace-moves-2026-11-11.md` (which itself ends up in
# `.mochi/evidence/handoff/`). This script never deletes anything: it only runs `mkdir -p`,
# `mv`, and writes the map and the new README.md (the old README.md is moved, not overwritten).
#
# Idempotent: an item whose destination exists and whose source is gone is reported as done.
# It refuses (exit 2, nothing moved) when
#   - a source is missing and its destination does not exist either (`--allow-missing` skips those),
#   - both source and destination exist,
#   - something listens on 3000, 8000 or 4184-4186 (dev/preview servers use these projects),
#   - a cargo or rustc process runs (the Cargo target moves),
#   - a source is on another filesystem than the workspace, or holds a mount point.
#
# Usage: scripts/workspace/migrate.sh [--workspace DIR] [--allow-missing] [--dry-run | --apply]
# DIR defaults to `.mochi` at the repository root that holds this script.
set -euo pipefail

STAMP="2026-11-11"
APPLY=0
ALLOW_MISSING=0
WORKSPACE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --apply) APPLY=1 ;;
    --dry-run) APPLY=0 ;;
    --allow-missing) ALLOW_MISSING=1 ;;
    --workspace) shift; WORKSPACE="${1:?--workspace needs a directory}" ;;
    -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
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

# Every move, relative to the workspace: "old|new". Loose files are listed one by one so that
# nothing unexpected is ever swept up by a glob.
MOVES=(
  "trials|evidence/trials"
  "trials-clone-dedupe-log.json|evidence/trials-clone-dedupe-log.json"
  "source-packs|evidence/source-packs"
  "previous-projects|evidence/previous-projects"
  "semantic-conversion|evidence/semantic-conversion"
  "references|evidence/references"
  "qualification|evidence/qualification"
  "mochi-exports|generated/exports"
  "cli-validation|generated/authored"
  "parity-runs|generated/parity-runs"
  "verification|runs/verification"
  "verification-gpu-disabled|runs/verification-gpu-disabled"
  "template-deps|runs/template/deps"
  "template-upgrade|runs/template/upgrade"
  "bun-browser-diagnose.log|runs/logs/bun-browser-diagnose.log"
  "bun-browser-tests.log|runs/logs/bun-browser-tests.log"
  "bun144-browser-tests.log|runs/logs/bun144-browser-tests.log"
  "check.log|runs/logs/check.log"
  "examples-results.json|runs/logs/examples-results.json"
  "final-browser-tests.log|runs/logs/final-browser-tests.log"
  "final-clippy.log|runs/logs/final-clippy.log"
  "final-release-build.log|runs/logs/final-release-build.log"
  "final-rust-tests.log|runs/logs/final-rust-tests.log"
  "infinitegarden-production.log|runs/logs/infinitegarden-production.log"
  "infinitegarden-rebuild.log|runs/logs/infinitegarden-rebuild.log"
  "microsoft-ai-production.log|runs/logs/microsoft-ai-production.log"
  "microsoft-ai-rebuild.log|runs/logs/microsoft-ai-rebuild.log"
  "natural-export.log|runs/logs/natural-export.log"
  "natural-production.log|runs/logs/natural-production.log"
  "natural-rebuild.log|runs/logs/natural-rebuild.log"
  "package-files.txt|runs/logs/package-files.txt"
  "package-list.log|runs/logs/package-list.log"
  "structure-tests.log|runs/logs/structure-tests.log"
  "template-build.log|runs/logs/template-build.log"
  "rebuild-examples.py|runs/scripts/rebuild-examples.py"
  "verify-examples.py|runs/scripts/verify-examples.py"
  "target|cache/target"
  "handoff|evidence/handoff"
  "README.md|evidence/handoff/README-before-$STAMP.md"
)
# Left where they are on purpose (anything else at the top level is reported, not moved).
STAY=(projects .claude .DS_Store evidence generated runs cache cli-release cli-final)

fail=0
problem() { echo "refusing: $*" >&2; fail=1; }

device() { stat -f %d "$1" 2>/dev/null || stat -c %d "$1"; }

# A destination parent that exists must be a real directory on the workspace's filesystem:
# a symlinked `evidence` would send `mv` outside the workspace, and another volume would make it
# copy and then delete. Prints the problem and returns 1.
check_dest() {
  local new="$1" cur="$M" part
  local -a parts
  IFS=/ read -r -a parts <<< "$(dirname "$new")"
  for part in "${parts[@]}"; do
    [ "$part" = "." ] && continue
    cur="$cur/$part"
    if [ -L "$cur" ]; then echo "destination component is a symlink: ${cur#"$M"/}" >&2; return 1; fi
    if [ -e "$cur" ]; then
      [ -d "$cur" ] || { echo "destination component is not a directory: ${cur#"$M"/}" >&2; return 1; }
      [ "$(device "$cur")" = "$WS_DEV" ] || { echo "destination is on another filesystem: ${cur#"$M"/}" >&2; return 1; }
    fi
  done
  return 0
}

# --- preflight: state of every item ------------------------------------------------------------
PENDING=()
DONE=0
WS_DEV="$(device "$M")"
for pair in "${MOVES[@]}"; do
  old="${pair%%|*}"; new="${pair#*|}"
  # Order matters: `handoff` moves before README.md goes into it, so a destination
  # directory is never pre-created by an earlier move.
  src="$M/$old"; dst="$M/$new"
  if [ "$old" = "README.md" ] && grep -q "Layout version: $STAMP" "$src" 2>/dev/null; then
    DONE=$((DONE + 1))   # already the new README
  elif [ -e "$src" ] || [ -L "$src" ]; then
    if [ -e "$dst" ] || [ -L "$dst" ]; then
      problem "both exist, not overwriting: $old and $new"
    else
      PENDING+=("$pair")
    fi
  elif [ -e "$dst" ] || [ -L "$dst" ]; then
    DONE=$((DONE + 1))
  elif [ "$ALLOW_MISSING" -eq 1 ]; then
    echo "note: missing, skipped: $old"
  else
    problem "missing: $old (and no $new); pass --allow-missing to skip"
  fi
done

# Same filesystem, no nested mount points: a cross-device mv would copy and then delete.
for pair in ${PENDING[@]+"${PENDING[@]}"}; do
  old="${pair%%|*}"; src="$M/$old"
  check_dest "${pair#*|}" || fail=1
  if [ "$(device "$src")" != "$WS_DEV" ]; then
    problem "on another filesystem than the workspace: $old"
  elif [ -d "$src" ] && [ ! -L "$src" ]; then
    devs="$(find "$src" -type d -exec sh -c 'for d; do stat -f %d "$d" 2>/dev/null || stat -c %d "$d"; done' _ {} + | sort -u | wc -l | tr -d ' ')"
    [ "$devs" = "1" ] || problem "holds a mount point (several devices): $old"
  fi
done

# Servers and builds that use these directories.
# lsof exits 1 both for "nothing found" and for some errors, so check that it works at all, and
# also try to connect.
busy=""
if ! command -v lsof >/dev/null 2>&1 || ! lsof -nP -p $$ >/dev/null 2>&1; then
  problem "lsof is missing or not working, cannot check the ports"
fi
for port in 3000 8000 4184 4185 4186; do
  if { command -v nc >/dev/null 2>&1 && nc -z 127.0.0.1 "$port" >/dev/null 2>&1; } \
    || [ -n "$(lsof -nP -iTCP:"$port" -sTCP:LISTEN -t 2>/dev/null || true)" ]; then
    busy="$busy $port"
  fi
done
[ -z "$busy" ] || problem "something is listening on port(s):$busy (stop the dev/preview servers first)"
if pgrep -x cargo >/dev/null 2>&1 || pgrep -x rustc >/dev/null 2>&1; then
  problem "a cargo or rustc process is running; the Cargo target moves to cache/target"
fi

# Unplanned top-level entries: reported only.
for path in "$M"/* "$M"/.[!.]*; do
  [ -e "$path" ] || [ -L "$path" ] || continue
  name="$(basename "$path")"
  known=0
  for pair in "${MOVES[@]}"; do [ "${pair%%|*}" = "$name" ] && known=1; done
  for s in "${STAY[@]}"; do [ "$s" = "$name" ] && known=1; done
  [ "$known" -eq 1 ] || echo "note: not in the plan, left alone: $name"
done

if [ "$fail" -ne 0 ]; then
  echo "nothing was moved." >&2
  exit 2
fi

# --- report ---------------------------------------------------------------------------------------
echo "workspace: $M"
for pair in ${PENDING[@]+"${PENDING[@]}"}; do
  echo "  ${pair%%|*} -> ${pair#*|}"
done
echo "$((${#PENDING[@]})) to move, $DONE already done."
if [ "$APPLY" -ne 1 ]; then
  echo "dry run: nothing changed. Re-run with --apply to move."
  exit 0
fi
# --- apply -----------------------------------------------------------------------------------------
write_readme() {
  if [ ! -e "$M/README.md" ] && [ ! -L "$M/README.md" ]; then
    cat > "$M/README.md" <<EOF
# Local Mochi workspace

Git-ignored, excluded from crate packages. Layout version: $STAMP. Rules: docs/WORKSPACE.md.

- \`evidence/\`: never deleted automatically (captures in \`trials/\`, \`source-packs/\`, \`previous-projects/\`,
  \`semantic-conversion/\`, \`references/\`, \`qualification/\`, \`handoff/\`).
- \`projects/\`: hand-edited canonical TanStack projects.
- \`generated/\`: \`exports/\`, \`authored/\`, \`parity-runs/\` (prune old runs with \`mochi clean --runs\`).
- \`runs/\`: logs, scripts, verification output. Put new scratch files here.
- \`cache/target/\`: Cargo build cache (\`.cargo/config.toml\`).

The moves that produced this layout are in \`evidence/handoff/workspace-moves-$STAMP.md\`.
EOF
    echo "wrote README.md"
  fi
}
if [ "${#PENDING[@]}" -eq 0 ]; then
  # Interrupted after the old README moved but before the new one was written: finish that.
  write_readme
  echo "nothing more to move."
  exit 0
fi

# The map goes to the handoff directory, wherever it currently is.
if [ -d "$M/handoff" ]; then MAPDIR="$M/handoff"; else MAPDIR="$M/evidence/handoff"; fi
mkdir -p "$MAPDIR"
MAP="$MAPDIR/workspace-moves-$STAMP.md"
{
  echo
  echo "## Run $(date '+%Y-%m-%d %H:%M:%S')"
  echo
  echo "Moves (relative to \`$M\`):"
  echo
  for pair in "${PENDING[@]}"; do echo "- \`${pair%%|*}\` -> \`${pair#*|}\`"; done
  echo
  echo "Reversal (run from \`$M\`, in this order; then \`rmdir\` any now-empty \`evidence\` \`generated\` \`runs\` \`cache\` directories):"
  echo
  echo '```sh'
  i=${#PENDING[@]}
  while [ "$i" -gt 0 ]; do
    i=$((i - 1))
    pair="${PENDING[$i]}"
    if [ "${pair%%|*}" = "README.md" ]; then
      echo "mv -n 'README.md' 'README-layout-$STAMP.md'   # the new README is in the way"
    fi
    echo "mv -n '${pair#*|}' '${pair%%|*}'"
  done
  echo '```'
} >> "$MAP"
echo "map: $MAP"

for pair in "${PENDING[@]}"; do
  old="${pair%%|*}"; new="${pair#*|}"
  check_dest "$new" || { echo "error: unsafe destination, not moving: $new (see $MAP)" >&2; exit 3; }
  mkdir -p "$(dirname "$M/$new")"
  if [ -e "$M/$new" ] || [ -L "$M/$new" ]; then
    echo "error: destination appeared, not moving: $new (see $MAP)" >&2
    exit 3
  fi
  mv -n "$M/$old" "$M/$new"
  if [ -e "$M/$old" ] || [ ! -e "$M/$new" ]; then
    echo "error: move did not complete: $old -> $new (see $MAP)" >&2
    exit 3
  fi
  echo "moved $old -> $new"
done

# The new README (the old one now sits in evidence/handoff/).
write_readme
echo "done."
