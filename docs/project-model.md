# Deterministic project model

The development checkout implements `mochi model-project <clone>` after a source-evidence
capture. It writes `reconstruction/project.json` without network or AI calls. This artifact
is the shared input for the [first TanStack emitter](export.md), future adapters and semantic interpretation; it is not
an executable frontend or recovered original source.

```sh
mochi clone https://example.com -o captured --source-evidence --max-pages 20
mochi check-evidence captured
mochi model-project captured
```

## Records and provenance

Schema version 1 uses algorithm `mochi-source-model-v1`. Maps are sorted and identities use
SHA-256. Identical frozen inputs produce identical output bytes; there are no timestamps.
The input fingerprint covers capture paths, raw capture digests, normalized capture content,
and asset records including any verified local file hashes. These hashes detect changes;
they are not signatures proving authenticity of edited files.

| Record | Meaning |
| --- | --- |
| `routes` | Exact HTTP(S) URL, pathname and query; fragments remain in observations/navigation. Query order is preserved. |
| `observations` | Original snapshot path/digest, route reference, complete ordered capture and mapping from local style indices to shared style IDs. Metadata, screenshots, CSSOM, fonts and control states stay here. |
| `styles` | Exact computed declaration maps addressed by content hash, including observed custom properties. No invented token names or rounded values. |
| `components` | Structural candidates, neutral names, prototype source references, prop targets, instance values and style-variant hashes. |
| `layouts` | Exact shared siblings before/after a single direct `<main>` child of `<body>`, seen on at least two distinct routes. These are shell candidates, not router nesting. |
| `navigation` | Observed anchor/area href, resolution against the recorded base URL and an optional exact captured-route reference. Uncaptured destinations remain unresolved. |
| `assets` | Exact URLs/usages and external, unresolved, ambiguous or local-candidate status. Original runtime requests remain observations. |

Full captures are embedded so downstream stages can retain unsupported nodes and consult
CSS conditions, font descriptors, namespaces and pseudo styles. The shared style map deduplicates
lookup records; it does not remove any captured evidence. Geometry remains per observation.

## Candidate algorithm

1. Compute ordered subtree hashes bottom-up. Tags, namespaces, attribute names, selected
   semantic attribute values, control presence and text positions must match exactly.
   Whitespace-only and nonempty text slots stay distinct. Content values and computed styles
   can differ; those differences are retained rather than claimed equivalent.
2. Consider element subtrees of 3–5,000 nodes, excluding `html`, `head`, `body` and `main`.
   For each route, count the maximum simultaneous occurrences across its observations.
   A single component moving between viewport positions cannot create reuse. Occurrence
   ranks are only a counting convention, not proof of identity across responsive layouts.
3. Rank by occurrence count, subtree size and stable hash. Select at most 256 candidates,
   preventing ancestor/descendant overlap within each observation. All remaining nodes stay
   available as inline evidence. No approximate matching or behavior inference occurs.
4. Align each candidate's nodes in depth-first child order. Varying full text, namespaced
   attribute values and live control records become explicit prop targets. Each instance
   stores its values. Semantic attribute changes such as role/type/ARIA state prevent
   alignment; computed-style differences produce explicit style-variant hashes. The full
   style maps remain available through each instance's observation and node references.

Prototype plus prop bindings reproduces instance **content**. Style, geometry and parent
indices still come from that instance's evidence. Generated names such as `Component1` and
`prop_1` are neutral fallbacks. The optional [interpretation stage](semantic-interpretation.md) can
propose fixed semantic roles over these candidates. It does not mutate this model or choose
new source identifiers. Source generation belongs to the separate export stage.

## Assets and bounds

Local candidates are checked beneath the clone's `site/` directory. Only exact observed
origins are considered local; external origins remain external. Available candidate files
get a size and SHA-256 hash. This proves which local bytes were read, not that an origin URL
uniquely produced those bytes. The mirror may already have localized CSS or removed trackers.
No original JavaScript bundle is selected as the runtime of a reconstructed application.

Mirror paths omit URL queries, so query-bearing URLs and URLs sharing one path remain
`ambiguous_mirror_path` even when a file exists. Explicit `src`/`poster`, stylesheet URLs,
resource timing and the mirror's conservative CSS URL discovery populate the registry.
This is not a complete CSS/srcset parser; untouched declarations and attributes remain in
observations for later emitter handling. Font declarations do not prove valid font binaries.

Existing source limits still apply: at most 1,000 captures and 250,000 total nodes. Model
building additionally caps aggregate raw/normalized source JSON at 64 MiB, output at 128 MiB,
asset URLs at 20,000 and candidate local bytes at 256 MiB total/64 MiB per file. An exceeded
budget fails the build; candidate selection limits leave evidence available inline.

The builder verifies manifests, graph integrity, completeness and screenshot hashes/dimensions;
it checks source digests again while reading and revalidates inventories/local assets before
the final atomic write. Missing, incomplete, unsafe or concurrently changing inputs preserve any previous
model. Source files and validation reports are not modified. Symlinks and traversal are refused;
this is not isolation against a process concurrently swapping local filesystem entries.

`Project::validate()` checks capture validity, source identities, route/style references,
input fingerprints and regenerated deterministic component/layout/navigation decisions.
Validation of an in-memory model does not reread its source files or local assets; use
`model-project` to rebuild and verify those on disk.
