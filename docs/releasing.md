# Releasing Mochi

A release has two publications: the `mochi-web` crate and a GitHub release for
the same version and source commit. The executable and library remain `mochi`.
Publishing the crate alone does not complete a release.

## Prepare the notes and source

Maintain [the next-release draft](releases/NEXT.md) while implementing changes.
Review the entire diff from the previous `mochi-web-v*` tag, including earlier
unreleased commits. Describe features and user-visible behavior, rather than
copying commit subjects. Include:

- New commands, framework support, source structure, styling, and behavior.
- Changed defaults, migration steps, and compatibility implications.
- Validation actually performed against the release commit.
- Remaining limitations, including model/provider work not qualified live.
- The exact package version, install command, source commit, and comparison link.

Keep unfinished work explicitly pending in the draft. Do not advertise trial
projects under ignored `.mochi/` directories as shipped CLI capabilities unless
the package contains the implementation that produces those results.

When implementation and validation are complete, choose the version, update
`Cargo.toml` and `Cargo.lock`, and promote the draft to
`docs/releases/<version>.md`. Remove draft markers only after replacing them with
observed results. Commit the version, notes, and implementation using Conventional
Commits. Leave unrelated local files out of the commit and package.

## Validate the release commit

Run formatting, Clippy, unit/integration tests, and the browser suites used by CI:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --test browser --test source_browser --test export_browser \
  --test recover_browser --test authored_browser -- --ignored
cargo publish --locked --dry-run
```

Use Bun to install generated projects with frozen locks, build and typecheck them,
and verify the affected routes and interactions in a browser. Release notes must
distinguish a fixture passing from a real site's full semantic conversion.
Inspect `cargo package --list` and the packaged `.cargo_vcs_info.json`; the archive
must identify the intended commit. Do not use `--allow-dirty` to bypass this check.
Push that commit and verify its required GitHub checks before publication.

## Publish and pair the release

Use locally configured credentials; do not put tokens in commands, notes, or logs.
Set these values from the finalized package and notes, then inspect them before
running the publication commands:

```sh
release_version=$(cargo metadata --no-deps --format-version 1 | \
  jq -r '.packages[] | select(.name == "mochi-web") | .version')
release_commit=$(git rev-parse HEAD)
release_tag="mochi-web-v${release_version}"
release_notes="docs/releases/${release_version}.md"
test -s "$release_notes"

cargo publish --locked
```

Verify that the registry exposes the exact version and that a fresh installation
of that version works. Keep verification output under `.mochi/`:

```sh
cargo install mochi-web --version "=${release_version}" --locked \
  --root ".mochi/releases/${release_version}/install"
".mochi/releases/${release_version}/install/bin/mochi" --version
```

Create the GitHub release at the same commit with the composed notes:

```sh
gh release create "$release_tag" --repo enkyuan/mochi \
  --target "$release_commit" --title "mochi-web ${release_version}" \
  --notes-file "$release_notes"
gh release view "$release_tag" --repo enkyuan/mochi
```

Verify the remote tag resolves to the published source commit, the release is
published, and its notes describe this version. Monitor the release-build workflow;
its build artifacts are separate from the crates.io package. Report build failures
explicitly instead of claiming binary availability.

If crates.io publication succeeds but the GitHub step fails, retry the GitHub step
for that same version and commit. Inspect any existing tag or draft first; do not
republish under a different version or replace a mismatched tag to hide the failure.
If a matching GitHub release already exists, update its body from the reviewed
versioned notes rather than creating a duplicate.

The final release report links both the crates.io version and GitHub release and
states the registry-install and release-build results.
