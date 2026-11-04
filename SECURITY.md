# Security policy

## Reporting a vulnerability

Please don't open a public issue. Report it privately through
[GitHub security advisories](https://github.com/eshaangulati123/perfectclone-website-cloner/security/advisories/new).
Include steps to reproduce and the version (`perfectclone --version`). You'll get an
acknowledgement within a few days. A fix or mitigation will follow as soon as the issue is
confirmed, and you'll be credited unless you'd rather not be.

## Scope

Things worth reporting include:

- A crafted site that makes `perfectclone clone` write files outside the output folder
- `perfectclone serve` serving files outside the site folder
- Code execution while cloning or editing

## Safe use

`perfectclone serve` is a local preview server. It listens on 127.0.0.1 only and isn't meant
for production. Deploy clones to a static host instead.

A clone contains the original site's JavaScript, which runs in your browser when you preview
it. Clone sites you trust.
