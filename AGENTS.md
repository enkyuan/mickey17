# Repository instructions

- Every commit subject must use Conventional Commits: `type(scope): description`.
  Scope is optional; a breaking change may use `!` before the colon.
- Allowed types: `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`,
  `revert`, `style`, and `test`. Use the same format for merge/squash commit subjects
  and pull request titles.
- Enable the tracked commit-message hook with `git config core.hooksPath .githooks`.
  See [CONTRIBUTING.md](CONTRIBUTING.md) for examples.
- Every new `mochi-web` publication on crates.io must have a matching GitHub
  release, tagged `mochi-web-v<version>` at the published package commit.
  Compose reviewed feature, migration, limitation, and validation notes for the
  changes since the previous release. Follow [docs/RELEASING.md](docs/RELEASING.md);
  do not mark a release complete while either publication remains unverified.
