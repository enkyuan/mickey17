// Lifted by `mochi lift` from source.js (sha256 1dcdfd40213e).
// Bodies keep the original computation; names and signatures come from the
// reviewed lift specification. Per-function provenance is in math.lift.json.

/** Lifted from `clamp` (line 1, bytes 14..48, sha256 ec71c0efbafa). */
export function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

/** Lifted from `lerp` (line 3, bytes 74..102, sha256 4e7623c6fbec). */
export function mix(start: number, end: number, amount: number): number {
  return start + (end - start) * amount;
}

/** Lifted from `ease` (line 4, bytes 104..170, sha256 0d6326647849). */
export function easeIn(input: number): number {
  let clamped = clamp01(input);
  return mix(0, 1, clamped * clamped);
}
