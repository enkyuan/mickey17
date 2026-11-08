# Repository instructions

- Every commit subject must use Conventional Commits: `type(scope): description`.
  Scope is optional; a breaking change may use `!` before the colon.
- Allowed types: `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`,
  `revert`, `style`, and `test`. Use the same format for merge/squash commit subjects
  and pull request titles.
- Enable the tracked commit-message hook with `git config core.hooksPath .githooks`.
  See [CONTRIBUTING.md](CONTRIBUTING.md) for examples.
