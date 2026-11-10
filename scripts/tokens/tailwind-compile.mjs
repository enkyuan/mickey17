// Compile Tailwind v4 candidates against an input stylesheet, offline.
//   echo '{"css":"@theme static{...} @tailwind utilities;","candidates":["p-4"]}' \
//     | bun scripts/tokens/tailwind-compile.mjs <path to node_modules/tailwindcss>
// Prints the generated CSS. Exits 1 with the error on stderr when Tailwind rejects the input.
import { readFileSync } from 'node:fs';

const dir = process.argv[2];
if (!dir) {
  console.error('usage: tailwind-compile.mjs <tailwindcss package dir>  (JSON on stdin)');
  process.exit(2);
}
const { compile } = await import(`${dir}/dist/lib.mjs`);
const input = JSON.parse(readFileSync(0, 'utf8'));
try {
  const compiler = await compile(input.css, { base: process.cwd() });
  process.stdout.write(compiler.build(input.candidates));
} catch (error) {
  console.error(String(error && error.message ? error.message : error));
  process.exit(1);
}
