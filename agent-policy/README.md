# agent-policy

Pure, dependency-light policy library for the agent surface (see `docs/AGENT_SURFACE.md`
and `docs/agent-surface/log/agent-policy.md` for the threat model and decisions).

```bash
bun install
bun test          # 283 tests, incl. fast-check properties and the injection corpus
bun run check     # strict tsc --noEmit
bun run mutate    # one-edit mutants of security comparisons; the suite must go red for each
```

No decision function performs network I/O; callers pass DNS results, robots.txt text,
the clock and verified inputs in.
