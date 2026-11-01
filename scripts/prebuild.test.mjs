import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { COPIES } from './prebuild.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));

test('every vendored UI asset is provided by an installed npm package', { skip: !existsSync(join(root, 'node_modules', 'katex')) && 'run npm ci first' }, () => {
  for (const [from] of COPIES) {
    assert.ok(existsSync(join(root, 'node_modules', from)), `missing node_modules/${from}`);
  }
});

test('vendored assets land in distinct locations', () => {
  const targets = COPIES.map(([, to]) => to);
  assert.equal(new Set(targets).size, targets.length);
});
