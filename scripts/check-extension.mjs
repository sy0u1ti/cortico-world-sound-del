import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseExtensionManifest, EXTENSION_API_VERSIONS } from 'cortico/extensions/manifest.ts';
import { dryMountWorld } from 'cortico/extensions/dry-mount.ts';
import { checkDescribe } from 'cortico/extensions/describe-export.ts';
import { authorGuide, checkTags, declaredTags, fetchTagIndex } from 'cortico/extensions/author-guide.ts';

const root = fileURLToPath(new URL('../', import.meta.url));
const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
const parsed = parseExtensionManifest(pkg, 'en');
assert.ok(parsed.ok, JSON.stringify(parsed));
assert.equal(parsed.manifest.api, EXTENSION_API_VERSIONS.world);
assert.ok(existsSync(join(root, 'assets/icon.svg')));
mkdirSync(join(root, 'scratch'), { recursive: true });
const options = {
  scratchDir: mkdtempSync(join(root, 'scratch', 'extension-check-')),
  packageDir: root, language: 'en',
};
const definition = (await import('../dist/index.js')).default;
const report = await dryMountWorld(definition, options);
const description = await checkDescribe('world', definition, parsed.manifest.describe, options);
assert.deepEqual(report.failures, []);
assert.deepEqual(report.warnings, []);
assert.deepEqual(description.failures, []);
assert.deepEqual(description.warnings, []);
const index = await fetchTagIndex();
if (index.ok) {
  const tags = checkTags(pkg.name, declaredTags(pkg), index.index);
  assert.deepEqual(tags.unknown, []);
  assert.deepEqual(tags.official, []);
}
console.log(`Extension checks passed: ${pkg.name}@${pkg.version}, World API ${parsed.manifest.api}`);
console.log([...report.ok, ...description.ok].join('\n'));
console.log((await authorGuide(root, pkg)).join('\n'));
