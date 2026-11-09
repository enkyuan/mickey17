// Lifted by `mochi lift` from source.js (sha256 b0eb5bb0a4a1).
// Bodies keep the original computation; names and signatures come from the
// reviewed lift specification. Per-function provenance is in hazards.lift.json.
import type { Point2 } from './geometry-types';

/** Lifted from `describe` (line 1, bytes 0..81, sha256 9a52e4ae5f63). */
export function describe(e: number): string {
  const s = `n=${e}
	 indented line
		deeper`;
  return s;
}

/** Lifted from `joinParts` (line 7, bytes 100..144, sha256 e13da8419b72). */
export function joinParts(a: string, b: string): string {
  return a + 'x\
		' + b;
}

/** Lifted from `fact` (line 11, bytes 159..225, sha256 89509e208a27). */
export function factorial(n: number): number {
  return !(n > 1 && n <= 150) ? 1 : n * factorial(n - 1);
}

/** Lifted from `isEven` (line 14, bytes 227..305, sha256 17e896f28b0c). */
export function isEven(n: number): boolean {
  return !(n >= 1 && n <= 500) ? n === 0 : isOdd(n - 1);
}

/** Lifted from `isOdd` (line 17, bytes 306..382, sha256 bb7121af59f5). */
export function isOdd(n: number): boolean {
  return !(n >= 1 && n <= 500) ? false : isEven(n - 1);
}

/** Lifted from `spread` (line 20, bytes 398..454, sha256 8d774b4b640e). */
export function spread(p: readonly Point2[]): number {
  let x, y;
  ({ x, y } = p[0]);
  return x - y;
}

/** Lifted from `label` (line 25, bytes 470..518, sha256 41a2b2199bb3). */
export function label(flag: boolean, text: string): string {
  return flag ? text.toUpperCase() : text;
}

/** Lifted from `holes` (line 26, bytes 534..587, sha256 b422c5e4f4fa). */
export function holes(n: number): (number | undefined)[] {
  const a = [1, , 3];
  a[5] = n;
  return a;
}
