// State vocabulary shared by the inventory generator, the drivers and the reports.
import type { Collection } from './diff';

export interface Descriptor { tag: string; role: string; name: string; text: string; nth: number }
export type Step =
  | { op: 'scroll'; frac: number; y?: number } // wheel-driven scroll to a fraction of the reference's scrollable height; y is resolved at run time
  | { op: 'click'; target: Descriptor }
  | { op: 'hover'; target: Descriptor }
  | { op: 'focus'; target: Descriptor }
  | { op: 'press'; key: string }
  | { op: 'tabs'; count: number } // press Tab count times and record the focus order
  | { op: 'wait'; ms: number };

export type Phase = 'initial' | 'scroll' | 'interaction' | 'bottom';
export interface StateSpec {
  name: string;
  route: string;
  viewport: [number, number];
  reducedMotion: boolean;
  phase: Phase;
  steps: Step[];
  /** Fixed virtual clock: the page loads with timers frozen, runs for exactly this many ms, then animations are frozen at it. */
  clockMs?: number;
  /** Run the reference twice and diff it against itself (nondeterminism floor). */
  control?: boolean;
  tags: string[];
}
export interface Gap { id: string; area: string; detail: string }
export interface Inventory {
  schema: 1;
  generatedFrom: { capture: string; manifestSha256: string; generator: string };
  siteOrigins: string[];
  /** prefers-color-scheme recorded by the capture; both sides are loaded with it. */
  colorScheme: 'dark' | 'light';
  routes: { route: string; title: string; evidence: string[] }[];
  viewports: { name: string; size: [number, number]; reason: string }[];
  breakpoints: { px: number; feature: string; files: string[] }[];
  controls: Record<string, { kind: string; descriptor: Descriptor }[]>;
  graphics: Record<string, Record<string, number>>;
  states: StateSpec[];
  gaps: Gap[];
}
export interface SideRun {
  collection: Collection;
  focusOrder: Record<string, unknown>[];
  errors: string[];
  blocked: string[];
  stepErrors: string[];
  settled: boolean;
  reachedScrollY: number;
  clock?: { ms: number; frozen: unknown };
  timingMs: number;
  /** Calibration only: did the injected mutation change any resolved computed style of any element? */
  mutationChangedComputedStyle?: boolean;
  /** Calibration only, breakpoint shifts: media rules rewritten / sheets that could not be read. */
  mutationRules?: { rewritten: number; unreadable: number };
  /** Calibration only: first (element, property) pairs whose resolved style moved after the mutation. */
  mutationChangedProps?: string[];
  /** Calibration only: full-page pixels (canvas/video hidden) before vs after the mutation. */
  mutationPixels?: 'same' | 'changed' | 'unstable';
}
