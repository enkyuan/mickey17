#!/bin/sh
# usage: scripts/semantic/verify-project.sh <assembled project dir> <route>...
# Runs the three commands the assembled project must pass, then server-renders every route.
set -e
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$1"; shift
bun install --frozen-lockfile
bun run check
bun run build
bun "$root/scripts/semantic/ssr-check.mjs" "$@"
