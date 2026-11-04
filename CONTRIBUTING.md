# Contributing to perfectclone

Thanks for helping. perfectclone is MIT-licensed and built in the open. Bug reports, site
compatibility fixes, docs and new features are all welcome.

## Ways to help

- **Report a site that doesn't clone cleanly.** This is the most useful thing you can do. Open
  an issue with the URL, the command you ran, what looked wrong, and the `failures` and
  `skipped` sections of `manifest.json`.
- **Add framework support.** Next.js App Router payloads are handled today. Remix, Nuxt,
  SvelteKit, Astro islands and Gatsby each have their own hydration data and could use the same
  treatment (see `src/perfectclone/flight.py`).
- **Add tracker signatures** in `src/perfectclone/trackers.py` with a link showing the snippet.
- **Improve the docs**, especially for deploying clones to different static hosts.

Check the open issues first, and comment on one before starting a large change so we can agree
on the approach.

## Development setup

```bash
git clone https://github.com/eshaangulati123/perfectclone-website-cloner
cd perfectclone-website-cloner
python -m venv .venv && source .venv/bin/activate
pip install -e ".[dev]"
playwright install chromium
ruff check .
pytest
```

Run the CLI from your checkout with `perfectclone ...` or `python -m perfectclone ...`.

`scripts/compat.py` is the real-world check. It clones a set of sites built with different
frameworks, serves each clone, and compares it with the live site: pixel match, new console and
hydration errors, and missing files. It uses the network, so run it by hand before changes to
crawling or mirroring:

```bash
python scripts/compat.py                  # all sites
python scripts/compat.py huly.io          # just one
```

## Project layout

| File | What it does |
| --- | --- |
| `src/perfectclone/crawl.py` | Renders pages in Chromium, follows links, records every asset request |
| `src/perfectclone/mirror.py` | Downloads server HTML, assets and Next.js navigation payloads |
| `src/perfectclone/flight.py` | Decodes and rewrites the Next.js RSC flight payload |
| `src/perfectclone/edit.py` | `replace`, `rehost` and `localize`: URL-safe, payload-aware edits |
| `src/perfectclone/trackers.py` | Tracker signatures and removal |
| `src/perfectclone/serve.py` | Local static server: clean URLs, `/_next/image`, RSC payloads |
| `src/perfectclone/util.py` | Shared helpers |
| `src/perfectclone/cli.py` | Command line |
| `tests/` | Offline unit tests, one file per module |
| `scripts/compat.py` | Clone-and-compare check against real sites |

## Making a change

1. Fork the repo and create a branch from `main`.
2. Keep pull requests focused: one fix or feature each.
3. Add or update tests in `tests/` for behaviour changes. Tests must not hit the network. Build
   the HTML or payload they need inline, like the existing ones do.
4. If you fixed a real site, say which one in the PR and how you checked it (for example, that
   `perfectclone serve` shows it with no console or hydration errors).
5. Run `ruff check .` and `pytest` and make sure both pass.
6. Add a line to `CHANGELOG.md` under "Unreleased".

### Style

- Plain Python 3.9+, standard library plus Playwright. Discuss new dependencies in an issue
  first.
- Match the surrounding code. Comments explain *why* something non-obvious is done (a framework
  quirk, a host's behaviour), not what the next line does.
- Fail soft while cloning: one bad asset should land in `manifest.json`, not crash the run.

## Reporting bugs

Use the bug report template. Include your OS, Python version, `perfectclone --version`, the
exact command, and the full output. For security issues, see [SECURITY.md](SECURITY.md) and
don't open a public issue.

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). By taking part you agree to
uphold it.

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
