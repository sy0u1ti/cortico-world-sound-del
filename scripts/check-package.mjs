import { existsSync, readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import assert from 'node:assert/strict';

const root = fileURLToPath(new URL('../', import.meta.url));
const manifest = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
for (const path of ['dist/index.js', 'dist/index.d.ts', 'bin/win32-x64/sound-del-helper.exe', 'describe.json', 'README_zh.md', 'LICENSE']) {
  assert.ok(existsSync(join(root, path)), `Missing package file: ${path}`);
}
const definition = (await import('../dist/index.js')).default;
assert.equal(definition.id, 'sound-del');
assert.equal(definition.defaults().enabled, false);
assert.equal(manifest.cortico.api, 6);
const helper = readFileSync(join(root, 'bin/win32-x64/sound-del-helper.exe'));
assert.equal(helper.toString('ascii', 0, 2), 'MZ');
const pe = helper.readUInt32LE(0x3c);
assert.equal(helper.toString('ascii', pe, pe + 2), 'PE');
assert.equal(helper.readUInt16LE(pe + 4), 0x8664);
console.log(`Package ready: ${manifest.name}@${manifest.version}`);
console.log(`Helper SHA256: ${createHash('sha256').update(helper).digest('hex')}`);
