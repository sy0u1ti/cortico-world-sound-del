import { test } from 'node:test';
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { HelperClient } from '../../src/helper-client.js';
import { readJson, scratch, until, writeJson } from '../fixtures.js';

test('the native maximum-duration limit wins over continuing heartbeat leases', { timeout: 20000 }, async () => {
  const directory = scratch('duration-'), audio = join(directory, 'audio.json');
  writeJson(audio, { devices: { speaker: { muted: false, active: true } } });
  let desired = true;
  const helper = new HelperClient({
    dataDir: directory, maxMuteSeconds: 10, desired: () => desired, changed() {},
    extraArgs: ['--simulation-file', audio], expectedBackend: 'simulation',
  });
  try {
    await helper.start();
    await until(() => readJson(audio).devices.speaker.muted);
    await until(() => helper.status.suspended === 'maximum_duration', 12500);
    assert.equal(readJson(audio).devices.speaker.muted, false);
    assert.equal(helper.status.pendingRestore, 0);
    desired = false;
    helper.changed();
    await until(() => helper.status.suspended === null);
    desired = true;
    helper.changed();
    await until(() => readJson(audio).devices.speaker.muted);
  } finally { assert.equal(await helper.stop(), true); }
  assert.equal(readJson(audio).devices.speaker.muted, false);
});
