# Distribution plan

Status: decisions only. Nothing here is published or implemented, and no channel
below is released before qualification passes. [RELEASING.md](RELEASING.md) remains
the process for the `mochi-web` crate and GitHub release until it is updated for the
new channels.

## Decided

| Topic | Decision |
| --- | --- |
| Source of truth | GitHub Releases with prebuilt `mochi` binaries (macOS arm64 and x64, Linux x64 and arm64, Windows); one tag per version, as `AGENTS.md` already requires |
| Owner | `hiyoriOS` (GitHub org or user); repo, tap and release assets live under it. The repo is `enkyuan/mochi` today, so decide whether to transfer it before the first release so formula URLs, npm `repository` and the install script point at one place |
| Homebrew | own tap, repo `hiyoriOS/homebrew-tap`, installed as `brew install hiyoriOS/tap/mochi` (Homebrew lowercases tap names; either casing works). The formula points at the release tarballs. Start with the tap; Homebrew's main repository has notability requirements |
| Install script | `curl … \| sh` plus a PowerShell equivalent, for Linux and anyone without Homebrew. Binaries fetched this way avoid the macOS quarantine flag |
| npm scope | `@hiyorios/*` (new npm packages must be lowercase, so `@hiyoriOS/…` is not valid). The scope must be an npm org or user you own; publish with `--access public` |
| npm packages | `@hiyorios/mochi` (launcher that resolves the right binary), `@hiyorios/mochi-<platform>` (per-platform binaries, the esbuild/Biome pattern), `@hiyorios/mochi-agent` (agent surface, run by the skill with `bunx`). The `bin` name stays `mochi` |
| crates.io | `mochi-web` stays available for `cargo install` and `cargo binstall`; no longer the primary path. Keep the `AGENTS.md` rule: every crates.io publication has a matching GitHub release |
| Agent skill | a `SKILL.md` plus an MCP config that runs `@hiyorios/mochi-agent` with `bunx`; no Homebrew involved. See [AGENT_SURFACE.md](AGENT_SURFACE.md) |
| Agent surface language | TypeScript; no new crate for it |

Channel priority: GitHub Releases, Homebrew tap, install script, then npm (when
`mochi-agent` ships), then winget/Scoop, Nix and distro packages on demand.

## To do before the first release

- Check that `hiyoriOS` (GitHub), `hiyorios` (npm scope) and `hiyoriOS/homebrew-tap`
  are available or owned, and that `brew search mochi` shows no clash.
- Decide the repo transfer to `hiyoriOS`.
- Release pipeline: build the platform matrix, upload assets with checksums, update
  the tap formula and npm packages from one tag. `cargo-dist` can generate much of
  this; confirm its current feature set before adopting it.
- One version across the binary, the npm packages and the tap; `mochi-agent` states
  which `mochi` versions it works with.
- Signing: Apple Developer account for signing and notarization (browser downloads
  otherwise trigger Gatekeeper), Windows SmartScreen reputation, npm provenance.
- Update `RELEASING.md` and `AGENTS.md` for the added channels, and verify a fresh
  install from each channel before reporting it available.
