import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { dryMountWorld, fakeWorldContext } from 'cortico/extensions/dry-mount.ts';
import { parseExtensionManifest } from 'cortico/extensions/manifest.ts';
import { exportDescribe } from 'cortico/extensions/describe-export.ts';
import definition from '../src/index.js';
import { ROOT, scratch } from './fixtures.js';

test('the real Cortico API 6 manifest validator accepts the package', () => {
  const manifest = JSON.parse(readFileSync(join(ROOT, 'package.json'), 'utf8'));
  const parsed = parseExtensionManifest(manifest);
  assert.equal(parsed.ok, true, JSON.stringify(parsed));
  assert.deepEqual(manifest.os, ['win32']);
  assert.deepEqual(manifest.cpu, ['x64']);
});

test('the default marketplace summary uses the Chinese locale copy', () => {
  const manifest = JSON.parse(readFileSync(join(ROOT, 'package.json'), 'utf8'));
  assert.equal(manifest.description, manifest.cortico.locales.zh.description);
  assert.match(manifest.description, /[\u4e00-\u9fff]/);
});

test('the real Cortico dry mount validates configuration and declarations without starting audio', async () => {
  const directory = scratch('contract-');
  const report = await dryMountWorld(definition, { scratchDir: directory, packageDir: ROOT });
  assert.deepEqual(report.failures, []);
  assert.deepEqual(report.warnings, []);
  const world = definition.create(fakeWorldContext(definition, { scratchDir: directory }));
  assert.deepEqual(world.tools(), []);
  assert.equal(await world.envPromptVars(), null);
  assert.equal(world.webUi?.('zh').lamps?.[0]?.state, 'offline');
  assert.equal(definition.defaults().enabled, false);
});

test('market description is generated from the same definitions', async () => {
  const actual = await exportDescribe('world', definition, { scratchDir: scratch('describe-'), packageDir: ROOT });
  const expected = JSON.parse(readFileSync(join(ROOT, 'describe.json'), 'utf8'));
  assert.deepEqual(actual, expected);
});
