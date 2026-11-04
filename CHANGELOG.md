# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## Unreleased

- `clone`: mirror files scripts reference by path in any folder, including `./` and `?v=` paths
  and `.wasm`, `.riv`, `.bin`, `.glb`, `.gltf` files.

## 0.1.0

- Published on PyPI: `pip install perfectclone`, or `uvx perfectclone` to run it without installing.
- Releases publish to PyPI automatically from GitHub releases.
- Requires Python 3.10+.
- `clone`: crawl with Chromium, mirror server HTML, every asset the pages load and Next.js
  navigation payloads, make the site's own asset URLs local (fonts and module scripts need
  this), strip trackers, write `manifest.json`. `--extract` saves text, metadata and screenshots.
- `serve`: local static server with clean URLs, byte ranges for video, `/_next/image` mapping
  and RSC payloads for Next.js client-side navigation.
- `replace` and `rehost`: URL-safe text edits that keep Next.js hydration intact.
- `strip-trackers`: remove analytics and ad pixels from HTML, Next.js payloads and runtime
  loaders in JS bundles.
- Chromium is downloaded automatically on first run.
- `scripts/compat.py`: clone-and-compare check against real sites.
