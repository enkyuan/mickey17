// Lifted by `mochi lift` from source.js (sha256 d206e9c0e012).
// Bodies keep the original computation; names and signatures come from the
// reviewed lift specification. Per-function provenance is in layout.lift.json.
export type Zone = { readonly span: number; readonly top: number };
export type Layout = { readonly key: string; readonly rows: Map<number, number[]>; readonly mid: number };

/** Captured controller state. Explicit inputs that replace captured variables of the original controller. */
export type Inputs = {
  /** Written by `resize`. Captured `w`. */
  readonly width: number;
  /** Written by `resize`. Captured `h`. */
  readonly height: number;
  /** Captured `table`. */
  readonly table: readonly (readonly number[])[];
  /** Captured `settings.bias`. */
  readonly bias: number;
  /** Captured `settings.gap`. */
  readonly gap: number;
  /** Captured `geom.config.size`. */
  readonly size: number;
  /** Sampled from the DOM. Captured `sample()`. */
  readonly zone: Zone;
};

/** Lifted from `clamp` (line 1, bytes 14..48, sha256 ec71c0efbafa). */
export function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

/** Lifted from `lerp` (line 2, bytes 63..91, sha256 4e7623c6fbec). */
export function mix(start: number, end: number, amount: number): number {
  return start + (end - start) * amount;
}

/** Lifted from `scaleFor` (line 14, bytes 559..745, sha256 83309ef42397). */
export function fitScale(inputs: Inputs): number {
  let zone = inputs.zone, edge = inputs.height * inputs.bias;
  return Math.min(zone.span * inputs.size * .85, inputs.width * (inputs.width < 760 ? .42 : .2), (edge - zone.top) * .98) / inputs.size;
}

/** The memoized computation of `rowsFor`, extracted so the cache can be dropped. Lifted from `layout` (line 18, bytes 746..1120, sha256 1f337657cbf8). */
export function computeLayout(inputs: Inputs): Layout {
  let key = `${inputs.width}x${inputs.height}`;
  let scale = fitScale(inputs), rows = inputs.table.map((r) => r.map((v) => mix(v, inputs.width, clamp01(scale)))), top = Infinity;
  for (let r of rows) for (let v of r) top = Math.min(top, v);
  return {
    key: key,
    rows: new Map([[1, rows[0]], [2, rows[1]]]),
    mid: .5 * inputs.height + top - inputs.gap
  };
}

/** Lifted from `layout` (line 18, bytes 746..1120, sha256 1f337657cbf8). */
export function rowsFor(inputs: Inputs): Map<number, number[]> {
  const layout = computeLayout(inputs);
  return layout.rows;
}

/** Lifted from `mid` (line 31, bytes 1121..1168, sha256 7714922d0990). */
export function midline(inputs: Inputs): number {
  return rowsFor(inputs), computeLayout(inputs).mid;
}
