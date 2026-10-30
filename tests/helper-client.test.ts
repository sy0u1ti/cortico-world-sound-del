import { test } from 'node:test';
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import type { ChildProcessWithoutNullStreams } from 'node:child_process';
import { HelperClient } from '../src/helper-client.js';
import { ROOT, readJson, scratch, until } from './fixtures.js';

function setup(mode: string) {
  const directory = scratch('client-'), trace = join(directory, 'trace.json');
  const helper = new HelperClient({
    dataDir: directory, maxMuteSeconds: 10, desired: () => true, changed() {},
    executable: process.execPath, expectedBackend: 'simulation',
    extraArgs: [join(ROOT, 'tests', 'helper-fixture.mjs'), mode, trace],
  });
  return { helper, trace };
}

test('a helper that consumes leases but stops reporting progress is shut down', async () => {
  const { helper, trace } = setup('silent');
  try {
    await helper.start();
    await until(() => helper.error !== null, 4_000);
    assert.match(helper.error!, /progress/);
    assert.equal(helper.ready, false);
  } finally { await helper.stop(); }
  const messages = readJson(trace);
  assert.equal(messages.at(-1).type, 'shutdown');
});

test('overflow is terminal: the pipe and retained partial frame are discarded', async () => {
  const { helper } = setup('flood');
  try {
    await helper.start();
    await until(() => helper.error !== null);
    assert.match(helper.error!, /size limit/);
    const child = (helper as unknown as { child: ChildProcessWithoutNullStreams }).child;
    assert.equal(child.stdout.destroyed, true);
    assert.equal(child.stdout.listenerCount('data'), 0);
    for (let n = 0; n < 128; n++) child.stdout.emit('data', Buffer.alloc(64 * 1024, 0x78));
    assert.equal(helper.ready, false);
    assert.equal(helper.status.recoveryUncertain, true);
  } finally { assert.equal(await helper.stop(), false); }
});

test('the output limit applies to frames, not a batch of valid frames', async () => {
  const { helper } = setup('batch');
  try {
    await helper.start();
    await until(() => helper.status.owned === 1);
    await until(() => helper.status.owned === 0);
    assert.equal(helper.error, null);
    await delay(500);
    assert.equal(helper.ready, true);
  } finally { assert.equal(await helper.stop(), true); }
});

test('split UTF-8 stream frames are assembled within the limit', async () => {
  const { helper } = setup('truncated');
  try {
    await helper.start();
    await until(() => helper.status.owned === 1);
    await until(() => helper.status.owned === 0);
    assert.equal(helper.status.suspended, '\u6b63\u5e38');
    assert.equal(helper.error, null);
  } finally { assert.equal(await helper.stop(), true); }
});

for (const mode of ['bad-status', 'duplicate-ready', 'bad-sequence', 'bad-latency']) {
  test(`invalid protocol fails closed without renewed mute leases: ${mode}`, async () => {
    const { helper, trace } = setup(mode);
    try {
      await helper.start();
      await until(() => helper.error !== null);
      assert.equal(helper.ready, false);
    } finally { assert.equal(await helper.stop(), false); }
    assert.equal(readJson(trace).at(-1).type, 'shutdown');
  });
}

test('repeated stop calls share the same bounded shutdown', async () => {
  const { helper } = setup('good');
  await helper.start();
  const first = helper.stop(), second = helper.stop();
  assert.equal(first, second);
  assert.equal(await first, true);
});

test('slow operation diagnostics are bounded and the failure record names the blocked stage', async () => {
  const { helper, trace } = setup('slow-failure');
  try {
    await helper.start();
    await until(() => helper.error !== null);
    assert.match(helper.error!, /journal\.save/);
    assert.equal(helper.timings.length, 32);
    const record = join(trace, '..', 'last-failure.json');
    await until(() => {
      try { return readJson(record).timings?.length === 32; } catch { return false; }
    });
    const written = readJson(record);
    assert.equal(written.timings.length, 32);
    assert.equal(written.timings.at(-1).ms, 64);
    assert.doesNotMatch(JSON.stringify(written), /transcript|speaker|premuted/);
  } finally { await helper.stop(); }
});

test('slow but successful calls leave diagnostic evidence without disabling the helper', async () => {
  const { helper, trace } = setup('slow-success');
  try {
    await helper.start();
    const record = join(trace, '..', 'last-slow-operation.json');
    await until(() => {
      try { return readJson(record).timings.at(-1)?.ms === 400; } catch { return false; }
    });
    assert.equal(helper.error, null);
    assert.equal(helper.ready, true);
    assert.equal(readJson(record).timings[0].phase, 'audio.read');
  } finally { assert.equal(await helper.stop(), true); }
});

test('completion timing is bounded, persisted off the hot path, and labelled as simulation', async () => {
  const { helper, trace } = setup('latency');
  try {
    await helper.start();
    await until(() => helper.latencies.length === 32);
    assert.equal(helper.latencies.at(-1)?.allMutedMs, 49);
    assert.equal(helper.latencies[0].allMutedMs, 18);
    const record = join(trace, '..', 'last-latency.json');
    await until(() => {
      try { return readJson(record).latencies?.length > 0; } catch { return false; }
    });
    const saved = readJson(record);
    assert.equal(saved.backend, 'simulation');
    assert.equal(saved.latencies[0].source, 'simulation');
    assert.doesNotMatch(JSON.stringify(saved), /transcript|speaker|premuted|keyCode/);
    assert.equal(helper.error, null);
  } finally { assert.equal(await helper.stop(), true); }
});
