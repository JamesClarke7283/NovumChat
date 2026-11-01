// Prepares the UI assets before Trunk builds the WebAssembly frontend:
//   1. copies browser libraries and fonts from node_modules into
//      novumchat-frontend/public/vendor (bundled into the app, works offline)
//   2. compiles Tailwind CSS (novumchat-frontend/input.css → public/css/app.css)
// Trunk runs this as a pre_build hook; `npm run prebuild:ui` runs it by hand.
import { spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const modules = join(root, 'node_modules');
const ui = join(root, 'novumchat-frontend');
const vendor = join(ui, 'public', 'vendor');

const fonts = (pkg, files) => files.map((f) => [`@fontsource-variable/${pkg}/files/${f}`, `fonts/${f}`]);

export const COPIES = [
  ['katex/dist/katex.min.js', 'katex/katex.min.js'],
  ['katex/dist/katex.min.css', 'katex/katex.min.css'],
  ['katex/dist/fonts', 'katex/fonts'],
  ['katex/dist/contrib/mhchem.min.js', 'katex/mhchem.min.js'],
  ['@highlightjs/cdn-assets/highlight.min.js', 'hljs/highlight.min.js'],
  ['dompurify/dist/purify.min.js', 'purify.min.js'],
  ['mermaid/dist/mermaid.min.js', 'mermaid.min.js'],
  ...fonts('instrument-sans', [
    'instrument-sans-latin-wght-normal.woff2',
    'instrument-sans-latin-wght-italic.woff2',
    'instrument-sans-latin-ext-wght-normal.woff2',
    'instrument-sans-latin-ext-wght-italic.woff2',
  ]),
  ...fonts('bricolage-grotesque', [
    'bricolage-grotesque-latin-opsz-normal.woff2',
    'bricolage-grotesque-latin-ext-opsz-normal.woff2',
  ]),
  ...fonts('jetbrains-mono', [
    'jetbrains-mono-latin-wght-normal.woff2',
    'jetbrains-mono-latin-ext-wght-normal.woff2',
  ]),
];

function newer(src, dest) {
  if (!existsSync(dest)) return true;
  return statSync(src).mtimeMs > statSync(dest).mtimeMs;
}

export function copyVendor({ log = console.log } = {}) {
  if (!existsSync(join(modules, 'katex'))) {
    throw new Error('node_modules is missing. Run `npm ci` in the repository root first.');
  }
  let copied = 0;
  for (const [from, to] of COPIES) {
    const src = join(modules, from);
    const dest = join(vendor, to);
    if (!existsSync(src)) throw new Error(`missing ${from}; run \`npm ci\``);
    if (statSync(src).isDirectory() ? !existsSync(dest) : newer(src, dest)) {
      mkdirSync(dirname(dest), { recursive: true });
      cpSync(src, dest, { recursive: true });
      copied++;
    }
  }
  if (copied) log(`vendor: copied ${copied} asset(s)`);
}

export function buildCss({ env = process.env } = {}) {
  const local = join(modules, '.bin', process.platform === 'win32' ? 'tailwindcss.cmd' : 'tailwindcss');
  const bin = existsSync(local) ? local : 'tailwindcss';
  const args = ['-i', join(ui, 'input.css'), '-o', join(ui, 'public', 'css', 'app.css')];
  if (env.TRUNK_PROFILE !== 'debug') args.push('--minify');
  const result = spawnSync(bin, args, { cwd: root, stdio: 'inherit', shell: process.platform === 'win32', env });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`tailwindcss exited with ${result.status}`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    copyVendor();
    buildCss();
  } catch (error) {
    console.error(`prebuild failed: ${error.message}`);
    process.exitCode = 1;
  }
}
