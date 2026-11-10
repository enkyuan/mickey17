// Tables parsed from the collector so there is one definition (style groups, implicit roles).
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
const src = readFileSync(resolve(import.meta.dir, '../../../scripts/parity/collect.js'), 'utf8');
const literal = (name: string) => {
  const m = new RegExp(`const ${name} = (\\{[\\s\\S]*?\\n?\\s*\\});`).exec(src);
  if (!m) throw new Error(`${name} not found in scripts/parity/collect.js`);
  return new Function(`return ${m[1]}`)();
};
export const GROUPS = { STYLE_GROUPS: literal('STYLE_GROUPS') as Record<string, string[]> };
export const IMPLICIT_ROLE: Record<string, string> = literal('IMPLICIT_ROLE');
