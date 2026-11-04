# Mosha

Clone a website's frontend with Chromium, then edit and serve the local copy. Written in Rust.

## Use

Requires Rust 1.99.1 and Chrome or Chromium. The executable is currently named `perfectclone`.

```sh
cargo install --path . --locked

perfectclone clone https://example.com -o example
perfectclone serve example
perfectclone replace example 'Example' 'Demo'
perfectclone rehost example https://example.com https://demo.example
perfectclone strip-trackers example
```

Clones contain `site/` and `manifest.json`. Use a new or empty output directory and check the
manifest for failed resources. `--extract` adds text, metadata and screenshots; `--max-pages`
defaults to 200. Trackers and robots rules can be overridden with `--keep-trackers` and
`--ignore-robots`. Set `PERFECTCLONE_CHROME` if browser discovery fails.

## Capabilities

- **Browser capture.** Scrolls to discover lazy assets; saves server HTML for hydration.
  Persistent background requests do not block document readiness.
- **Safe editing.** Rewrites Next.js Flight JSON and UTF-8 text lengths alongside HTML,
  protects asset URLs, and atomically replaces each edited file.
- **Bounded downloads.** Deduplicates up to 100,000 asset URLs, limits responses to 64 MiB,
  and fetches sequentially with a 30-second request timeout. Conflicting paths are reported.
- **Streaming preview.** Loopback server with clean URLs, RSC navigation, original-image
  routing, HEAD requests and byte ranges for media.

## Performance

Local release-build measurements on Apple M1, Rust 1.99.1:

| Workload | Time |
| --- | ---: |
| Replace text while protecting URLs, 156 KiB | 1.68 ms |
| Rewrite 4,096 Flight text rows | 923 µs |

These measure in-memory editing, not website cloning speed. The heap workload retained no
additional bytes after warm-up. [Validation](VALIDATION.md) has methodology, memory results
and commands to reproduce the checks.

## Limits

Frontends only: forms, APIs, embeds and external CDN assets may still need their original
servers. Authenticated or interaction-only resources may be missed. Tracker removal uses
signatures; arbitrary-site visual parity is not established.

Keep served files under your control. Path checks reject traversal and symlinks but do not
isolate concurrent local filesystem changes. Edits are atomic per file, not across a whole clone.

## License

[MIT](LICENSE).
