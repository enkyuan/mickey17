# Lifting functions from recovered controllers

`mochi lift` turns closed functions in a recovered JavaScript controller into
a typed TypeScript module with per-function provenance. It is one mechanism
inside semantic conversion, not a whole-page converter.

```sh
mochi lift-survey controller.js --start-line 2240 --end-line 2500   # what could be lifted, and why not
mochi lift spec.json --out curve-math.ts --manifest curve-math.lift.json
bun benchmarks/src/lift-differential.ts curve-math.lift.json curve-math.ts controller.js
bun benchmarks/src/lift-differential.ts hinge.lift.json hinge.ts controller.js --domain hinge.domain.ts
```

## What is liftable

A function is lifted only when every identifier it uses is a parameter, a
local, a global from a small allowlist, another liftable function, or a captured
value the specification turns into an explicit input (below). The allowlist is
by member, not by object: `Math.<pure function or constant>` (no `Math.random`),
`Number.<predicate, parser or constant>`, `Array.from/of/isArray`, calling
`Number(..)`, `Array(..)`, `isFinite`, `isNaN`, `new Map(..)`/`new Array(..)`, and
`Infinity`, `NaN`, `undefined`. Any other member of those objects (so
`Math.__memo`, `Number.cache`) and any bare use (`const M = Math`) blocks the
lift, and a module that assigns to an allowlisted global or to `Object`
(`Math.min = f`, `Object.prototype.x = 1`) is refused outright, because lifted code
would not see that state. For memo proofs the module is also refused if it uses
`Reflect`, `globalThis`, `__proto__`, `setPrototypeOf`, `.freeze` other than as the
callee `Object.freeze(`, passes `Object` around as a value or reaches it through a
member (`window.Object`, `x["Object"]`, `x["freeze"]`, `x["Reflect"]`); for any
context lift if it uses `eval`, `Function`, `.constructor.constructor` or the
computed keys `["constructor"]`/`["Function"]`. Patching the allowlisted globals
through other aliases or reflection (`const M = Math; M.min = f`,
`Object.defineProperty(Math, ..)`, `window.Math`) is **not detected**. `Map` was added because
construction and iteration are deterministic and a `new Map(..)` has no hidden
shared state; `Set`, `Object`, `Date`, `document` and the like stay blocked. Undeclared captured reads or writes, `this`,
undeclared globals, destructured or defaulted parameters, and `async`/generator
functions block the lift. `lift-survey` and the manifest's `unlifted` list
report the reason for each blocked function; nothing is guessed. `lift-survey`
never assumes a context, so it still reports every captured read as a blocker.

## Captured reads become explicit typed inputs

The specification may declare `contexts`: named bags of typed fields. A function
that names one (`"context": {"type": "...", "param": "inputs"}`) takes it as an
appended last parameter, and every captured read in its body is rewritten to a
field access.

| `from` | meaning | rewritten as |
| --- | --- | --- |
| `z` | a captured variable | `inputs.viewportWidth` |
| `y.finalCenterY`, `m.config.petalLength` | a static member path on a captured variable | the whole path becomes the field |
| `R()` | a zero-argument call of a captured function, e.g. a DOM sampler | the call becomes `inputs.zone` |

These checks are refusals for known patterns, not a soundness proof (see the caveats
in the memo section, which apply here too). The lifter verifies that

- every captured read is declared; a bare `y` when only `y.finalCenterY` is
  declared, a computed access, or a call with arguments is refused;
- a declared name stands for one *binding*: if the same name resolves to
  different variables in one function, or in different functions of the
  context, the lift is refused (the checker itself binds by name, so this is
  the lifter's job);
- declared captures are never written, and no member write or mutating call
  (`push`, `t["push"](..)`, `t.at(0)[0] = 1`, `delete`, `++`) targets one;
- inside a context function, any member write targets a local initialized with
  an identifier-free array, object or `Map` literal, never reassigned or
  redeclared, and never the target of a possibly-aliasing assignment
  (`q.a = table[0]`, destructuring or `for (q.a of ..)` targets); no helper it
  calls, at any depth, writes through anything that could be a parameter; a
  declared non-primitive input is not passed on as a bare value. Values *pushed*
  into such a local are not tracked, so `q.push(table[0]); q[0].push(1)` is a
  known gap;
- a function that calls a context-taking function (or itself) takes the same
  context, calls it with exactly the declared number of arguments (no spread), and
  the call gets the context appended; using it as a value is refused.

Known gap: a closure created inside a context function that outlives the call
(`return () => w * 2`) sees a snapshot of the inputs, where the original read the
variable live. Closures run during the call (`map`, `forEach`, IIFEs) are exact.

DOM access never appears in lifted code. A sampler such as `R()` stays unlifted
and reported; the lifted code receives what it returned as a typed input. The
differential supplies that input as sampled numbers.

## Memo caches

**What these checks are.** The lifter's static checks are a set of refusals for
known patterns, not a general soundness proof. A source that avoids every
pattern listed here can still defeat them (aliasing, reflection, code outside
the module). Each acceptance is conditional on what is written below. The Garden
hinge's stability rests on one input that is proven only if the global
`Object.freeze` was intact when the controller's literal was created (`y`) and on
two **reviewed assumptions, not proofs** (`I`, the imported ribs, and `m`, the
object `createFlowerGeometry` returns, plus the reviewer's word that
`m.config.upperHeight/lowerHeight` are numbers). The differential cannot test any
of this.


A function that memoizes into a captured variable is lifted only when its
shape is exactly `let KEY = <template>; if (!C || C.key !== KEY) { ..; C = { key: KEY, .. }; } .. C.field ..`
and the lifter proves, from the whole module, that dropping the cache cannot
change any output:

1. the memoizing function has no parameters; `KEY` is a template literal whose
   every interpolation is a bare declared field *annotated* `number`, joined by
   text (as the engine reads it, so `\x31` counts as `1`) containing a character that
   cannot occur in a printed number, and `KEY`
   itself is never reassigned, and the `key` stored in the cached object is that
   same variable (a different declaration with the same name inside the guarded
   block is refused). The `number` annotation is trusted: nothing here
   checks that the value really is a number (the differential does check its
   domain);
2. every reference to `C` in the module is the guard, the fill, a `C = null`
   reset, the memoizing function's final statement (which must be exactly
   `return C.field;`) or the second operand of a `memo_read` function's
   `return F(), C.field;`, so the cached object never reaches a helper; `C` is
   declared empty and once, with `let`/`const`, and is not exported; the guarded
   block has no `return`;
3. the computation (and its lifted dependencies) reads only declared inputs and
   has no cache of its own;
4. every input the key does not cover is stable, a reviewed assumption, or a
   coupled sampler:
   - *stable*: the root variable is declared once, never reassigned, and is a
     literal frozen with `Object.freeze` at every level over primitives (with the
     real global `Object`) and every declared path is an own data property of that
     literal ending in a primitive. This is the strongest thing the tool accepts:
     no property of such a value can change, **provided the global `Object.freeze`
     was intact when the literal was created**. The module is refused if it patches
     or aliases `Object.freeze`, uses `Reflect`/`__proto__`, or exports the value,
     but other modules the controller imports could still patch it before this one
     runs. The same direct checks as for reviewed roots (below) run on frozen roots;
   - *reviewed assumption*: any other root (an array literal, an import such as
     the ribs, an object built by an imported factory such as `m`) needs an entry
     under `memo.reviewed_assumptions` with a reason. It is **not proven**; the
     reason is recorded in the manifest, the assumption must list exactly the
     methods the source calls on the root (other than plain read methods such as
     `map`; methods that hand back elements such as `at`, `get`, `find`, `slice`,
     `values` must be listed), an assumption the tool did not need is an error,
     and `primitives` lists member paths the reviewer vouches are plain values
     (so reading them into a variable is not an alias). For such roots the lifter still refuses direct reassignment, member
     writes and mutating calls (including `t["push"]`, `t.at(0)[0] = 1`), inline
     callbacks that write, bare uses, prefixes of a declared path used as values,
     and chains passed as call or `new` arguments, spread, assigned, initialized,
     returned or stored in a literal (unless the leaf is a declared primitive), but
     it does not see aliases created in other ways, helpers, named callbacks,
     conditional expressions or accessors;
   - *sampler* (`R()`): its body is exactly `if (!U) { .. } return U;` with no
     parameters and no early return. Inside it, no mention of any function, class
     or object/array-literal binding defined in this module (so no `f()`, `new f`,
     `f```, `[f][0]()`, `util.f()`, getters, iterators or `toString` of such
     objects), and no unresolvable callee such as `(0, f)()`; **opaque captured
     bindings, for example DOM nodes obtained by a call, are not analysed** (the real
     `R` calls `getBoundingClientRect` on such nodes). A binding initialized or
     assigned from something that mentions a module function or object counts as
     module code too, followed transitively. Globals and DOM objects may also hold
     module functions installed elsewhere (`document.hook = f`), which is not detected.) It writes no declared input
     directly. `U` is declared empty and once with `let`/`const`, not exported, not
     edited in place (including through `F().x = v` or a local assigned from `F()`),
     named only by the guard, the fill, the final return and resets. It is filled
     only inside that guard, by a plain `=` of a **flat** plain-data literal (no
     accessors, methods, spreads, computed keys or nested objects/arrays, so it is
     never falsy and holds no live getter), not inside a nested function or class
     (no deferred refill; classes are refused in the sampler altogether), with each
     value written directly as arithmetic, a primitive literal, or a bare local of the
     sampler that only ever holds a number computed there (arithmetic, a literal or
     `Math.*`); any other expression, including globals and wrapped values, is
     refused.
     The result of `F()` is not passed on, returned or stored (callers may read it
     and keep it in a local). Every reset `U = null` is the directly adjacent
     comma-expression neighbour of a reset `C = null`, so a re-sample can never
     leave a stale `C`. Any other variable the sampler writes (the real `R` writes
     `P`) is listed in the manifest.

Not detected, and stated plainly: patching a standard builtin through an alias after
the cache is filled (`const M = Math; M.min = f`,
`Object.getPrototypeOf(new Map()).set = f`, `Object.defineProperty(Math, ..)`). The
original would keep serving its cached value while the lifted code recomputes with
the patched builtin.

The cache is replaced by an extracted pure computation (`compute_as`), and the
memoizing function becomes `const layout = computeLayout(inputs); ..layout.field..`.
A function that only reads the cache after calling the memoizing function
(`return Y(), L.hingeY`) is lifted with `memo_read`. If any step cannot be shown,
the function stays blocked with the reason. The manifest records the key,
every cache write, the stability evidence per input, the reviewed assumptions,
the coupled resets, the lines that call the memoizing function or a `memo_read` function, and what the
proof does **not** cover:

- for reviewed-assumption roots: aliases (`let r = ribs[0]; r.x = 1`), helpers and
  named callbacks that mutate them, accessors, and mutation by code or methods
  outside the module are the reviewer's responsibility;
- the original returns one shared cached object, the lifted function a fresh
  one; callers that mutate or identity-compare the result are listed, not analysed;
- `0` and `-0` print the same, so they share a key;
- the lifted code recomputes where the original cached, so it is a reference
  implementation for equivalence, not a performance drop-in.

## Hardening shared with closed lifts

Captured writes, writes to globals (`Math = null`, `Math.x = 1`), members of the
allowlisted globals that are not on the pure list, `.constructor`,
`.__proto__`, `.prototype` (also as string keys), `super`, `new.target`,
`import.meta`, and references to a binding that is reassigned or declared twice
all block a lift. Dynamic computed access (`x[name]`) cannot be analysed and is
not claimed to be. Named function expressions keep their self-reference.
Line breaks inside string and template literals are never re-indented.
Types, imports and declarations in a specification are parsed as TypeScript
and must be exactly one alias or one `import type`; `doc` text may not contain
`*/`; export names may not collide with globals, imports, declarations or
reserved words. The emitted module is re-parsed as TypeScript and refused if
TypeScript reads any original code differently (`a<b>(c)`), and both outputs
are written together or not at all. Typechecking the emitted module is not
done here (its imports point into the target project).

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
cases (`NaN`, infinities, `-0`) with exact equality (`Object.is`; `Map`s are
compared entry by entry in order). A parameter type with no input generator
fails; it is never skipped. Passing proves equivalence on the generated inputs,
not on every input.

Context-taking functions need `--domain <module.ts>`, which exports
`contexts` (a grid of real captured values per context type) and, for
memoizing functions, `sweeps` (ordered sequences). The checker rebuilds the
**original** text in a scope where each captured variable is bound to the same
value the lifted function receives, `name()` fields are functions returning the
sampled value, and each cache starts `null`. Every grid value gets a fresh
closure and is called twice (the second call is served by the original's cache);
each sweep replays one closure through the sequence so cache hits, key changes
and revisits occur. A sweep that revisits a key with different inputs is
refused, because the original would then serve a stale entry that the real
controller's paired resets prevent. The lifted function must also leave its
inputs untouched. A context type with no domain, or a memoizing function with
no sweep, fails.

Originals run in strict mode in a fresh `node:vm` realm per scope, and every call (original and lifted) runs as its own script with a 2 s limit: an input on which the original never finishes is reported as a failure, not waited on. Object key order and frozenness are compared, and what each side does to its ordinary arguments must match. A function whose original throws on more than 75% of the cases is not a pass. `TypeError`/`ReferenceError` messages are never compared (engine messages quote identifiers that renaming changes, so user-thrown ones are skipped too). A context field that never varies across the grid must be listed in the domain's `constants`, and a sweep must contain a cache hit, a key change and a revisit. The checker binds captured names by *name* (the lifter guarantees one binding per name); the lifted
module is re-imported (from the hashed bytes) per checked function. Errors must
have the same type and, except engine `TypeError`/`ReferenceError` messages that
quote identifiers, the same message; `throw null` and friends are compared too.
A function whose original never returns normally is not a pass. Domain values
must carry exactly the manifest's fields with the right primitive kinds.

Each function is checked in its own closure and the module's top-level code never runs, so writers elsewhere in the module (a resize handler, a poke function) and the original's own mutations of captured state are not exercised; the manifest is not hash-bound, so a hand-edited manifest can skip the memo checks. The checker cannot show the cache *coupling* or *stability* proofs: it stubs
`R()` with the supplied zone and replays a coherent controller, so a writer that
breaks those invariants is caught by the lifter's static rules or not at all.

The Garden hinge domain (`.mochi/source-packs/infinitegarden/lift/hero-hinge.domain.ts`)
checks the controller hash, that the rib data the controller imports equals the
project's `flower-ribs.ts`, that `y.petalLength`/`y.finalCenterY` are read from
the controller's frozen settings literal, and that the baseline
`createFlowerGeometry` reports that `petalLength`. The zone numbers are modelled
from `R()`'s formula with invented block positions plus synthetic stress zones;
they are not captured from a page. It perturbs `finalCenterY` and `petalLength`
so a field hard-coded to today's value would show.

## Status on the Garden hero

`B` (petal scale), `Y` (mirrored petal paths and memoized hinge) and `Q`
(`hingeY`) lift (see `.mochi/source-packs/infinitegarden/lift/hero-hinge.lift.json`).
The cache's guard, fill, resets and the `R`/`U` coupling are checked statically.
Stability of the inputs is **proven only for `y`** (a frozen literal, conditional on
an intact `Object.freeze`); for `I` (the imported ribs) and `m` (the object
`createFlowerGeometry` returns, with `m.geometry` called on it) it is a **reviewed
assumption, not a proof**, recorded in the manifest. `R` (DOM
sampler) is deliberately not lifted: it reads `document` and element geometry,
so its result is an input. What is *not* shown: that `R()` samples the same
numbers the domain supplies on a real page, and that the lifted hinge is what the
authored project uses; both need a browser comparison.
