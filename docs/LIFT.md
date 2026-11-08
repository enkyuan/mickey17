# Lifting functions from recovered controllers

`mochi lift` turns closed functions in a recovered JavaScript controller into
a typed TypeScript module with per-function provenance. It is one mechanism
inside semantic conversion, not a whole-page converter.

```sh
mochi lift-survey controller.js --start-line 2240 --end-line 2500   # what could be lifted, and why not
mochi lift spec.json --out curve-math.ts --manifest curve-math.lift.json
bun benchmarks/src/lift-differential.ts curve-math.lift.json curve-math.ts controller.js
```

## What is liftable

A function is lifted only when every identifier it uses is a parameter, a
local, a global from a small allowlist (`Math`, `Number`, `Array`, `Infinity`,
`NaN`, `undefined`, `isFinite`, `isNaN`) or another liftable function.
Reads or writes of captured variables, `this`, undeclared globals,
destructured or defaulted parameters, and `async`/generator functions block the
lift. `lift-survey` and the manifest's `unlifted` list report the reason for
each blocked function; nothing is guessed. Functions that cache or read DOM and
configuration state (for example a layout function that memoizes into a captured
variable) are out of scope until captured inputs can be made explicit.

## Specification

The reviewed specification names each function by its minified binding and
line, and supplies the semantic export name, TypeScript parameter and return
types, and optional local renames. `source_sha256` pins it to one source file.
`locals` keys are an original name (renames every binding of that name) or
`name#k` (the k-th binding of that name in the body, in source order).
Renames are applied through resolved scope information; a rename that would
collide with a kept identifier or merge two different identifiers is refused.
A function that depends on another must have that dependency named too.

## Provenance and validation

Each emitted function carries its original name, line, byte range and SHA-256.
The manifest records the same fields plus dependencies and module hash. The
differential checker re-slices the original text by those ranges (refusing a
changed span), evaluates it with only its own dependencies in scope, and
compares the original and lifted function on generated inputs, including edge
cases (`NaN`, infinities, `-0`) with exact equality. A parameter type with no
input generator fails; it is never skipped. Passing proves equivalence on the
generated inputs, not on every input.
