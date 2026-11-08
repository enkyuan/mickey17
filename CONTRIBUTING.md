# Contributing

Enable the repository's commit-message check after cloning:

```sh
git config core.hooksPath .githooks
```

Every commit subject uses Conventional Commits, including merge and squash commits:

```text
feat: add source project export
fix(capture): preserve query state
refactor: simplify the asset registry
chore: update development tools
docs: explain reconstruction limits
feat(export)!: change the output schema
```

Allowed types are `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`,
`revert`, `style`, and `test`. A scope is optional. Include a description after `: `.
Use the same format for pull request titles so squash merges keep the convention.
The commit-message hook checks local commits; the Commit conventions workflow checks
every commit reachable from the pushed branch or pull request head.

For crates.io publication and the corresponding GitHub release, follow
[Releasing Mochi](docs/RELEASING.md). The working notes for the next release are
in [docs/releases/NEXT.md](docs/releases/NEXT.md).
