import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { once } from 'node:events';
import { join } from 'node:path';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { HelperClient, HELPER_FILE } from '../../src/helper-client.js';
import { VoiceBridge } from '../../src/voice-bridge.js';
import { ROOT, readJson, scratch, until, voiceServer, writeJson, state } from '../fixtures.js';

function setupFiles() {
  const directory = scratch('native-');
  const audio = join(directory, 'audio.json');
  writeJson(audio, {
    devices: {
      speaker: { active: true, muted: false },
      'already-muted': { active: true, muted: true },
    },
  });
  return { directory, audio };
}

function rawHelper(directory: string, audio: string, extraArgs: string[] = []) {
  const child = spawn(HELPER_FILE, [
    '--data-dir', directory, '--simulation-file', audio, '--max-mute-seconds', '10', ...extraArgs,
  ], { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, shell: false });
  const messages: any[] = [];
  let buffer = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', (chunk: string) => {
    buffer += chunk;
    let index: number;
    while ((index = buffer.indexOf('\n')) !== -1) {
      messages.push(JSON.parse(buffer.slice(0, index)));
      buffer = buffer.slice(index + 1);
    }
  });
  child.stdin.on('error', () => {});
  child.stderr.resume();
  return {
    child, messages,
    exited: once(child, 'close'),
    lease(active: boolean) { child.stdin.write(JSON.stringify({ type: 'lease', active, captureId: active ? 'recording' : null }) + '\n'); },
  };
}

async function cleanup(child: ChildProcessWithoutNullStreams) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.stdin.end();
  await once(child, 'close');
}

test('native helper and real local voice transport complete an isolated recording cycle', async () => {
  const { directory, audio } = setupFiles();
  const server = await voiceServer();
  let helper: HelperClient;
  const bridge = new VoiceBridge({ origin: server.origin, password: () => '', changed: () => helper?.changed() });
  helper = new HelperClient({
    dataDir: directory, maxMuteSeconds: 10,
    desired: () => bridge.policy.active(performance.now()),
    changed() {}, extraArgs: ['--simulation-file', audio], expectedBackend: 'simulation',
  });
  try {
    await helper.start();
    bridge.start();
    await until(() => bridge.policy.snapshot !== null);
    server.gate(true);
    await until(() => readJson(audio).devices.speaker.muted);
    assert.equal(readJson(audio).devices['already-muted'].muted, true);
    assert.deepEqual(readJson(join(directory, 'recovery.json')).restoreUnmuted, ['speaker']);
    server.gate(false);
    await until(() => !readJson(audio).devices.speaker.muted);
    await until(() => readJson(join(directory, 'recovery.json')).restoreUnmuted.length === 0);
    assert.deepEqual(readJson(join(directory, 'recovery.json')).restoreUnmuted, []);
    assert.equal(readJson(audio).devices['already-muted'].muted, true);
  } finally {
    bridge.stop();
    assert.equal(await helper.stop(), true);
    await server.close();
  }
});

test('HTTP failure releases the actual helper, not just the TypeScript policy', async () => {
  const { directory, audio } = setupFiles();
  const server = await voiceServer();
  let helper: HelperClient;
  const bridge = new VoiceBridge({ origin: server.origin, password: () => '', changed: () => helper?.changed() });
  helper = new HelperClient({
    dataDir: directory, maxMuteSeconds: 10, desired: () => bridge.policy.active(performance.now()),
    changed() {}, extraArgs: ['--simulation-file', audio], expectedBackend: 'simulation',
  });
  try {
    await helper.start();
    bridge.start();
    await until(() => bridge.policy.snapshot !== null);
    server.gate(true);
    await until(() => readJson(audio).devices.speaker.muted);
    server.availability(false);
    await until(() => !readJson(audio).devices.speaker.muted);
  } finally { bridge.stop(); await helper.stop(); await server.close(); }
});

test('native lease expiry restores output even with stdin still open', async () => {
  const { directory, audio } = setupFiles();
  const process = rawHelper(directory, audio);
  try {
    await until(() => process.messages.some((m) => m.type === 'ready'));
    process.lease(true);
    await until(() => readJson(audio).devices.speaker.muted);
    await until(() => !readJson(audio).devices.speaker.muted, 4500);
    await until(() => process.messages.some((m) => m.suspended === 'lease_expired'));
    assert.ok(process.messages.some((m) => m.suspended === 'lease_expired'));
    process.lease(true);
    await new Promise((resolve) => setTimeout(resolve, 150));
    assert.equal(readJson(audio).devices.speaker.muted, false);
    process.lease(false);
    process.lease(true);
    await until(() => readJson(audio).devices.speaker.muted);
  } finally { await cleanup(process.child); }
  assert.equal(readJson(audio).devices.speaker.muted, false);
});

test('EOF restores and acknowledges completion', async () => {
  const { directory, audio } = setupFiles();
  const process = rawHelper(directory, audio);
  await until(() => process.messages.some((m) => m.type === 'ready'));
  process.lease(true);
  await until(() => readJson(audio).devices.speaker.muted);
  process.child.stdin.end();
  const [code] = await process.exited;
  assert.equal(code, 0);
  assert.equal(readJson(audio).devices.speaker.muted, false);
  assert.ok(process.messages.some((m) => m.type === 'stopped' && m.restored === true));
});

test('malformed stdin restores instead of holding the last active state', async () => {
  const { directory, audio } = setupFiles();
  const process = rawHelper(directory, audio);
  await until(() => process.messages.some((m) => m.type === 'ready'));
  process.lease(true);
  await until(() => readJson(audio).devices.speaker.muted);
  process.child.stdin.write('{"type":"lease","active":"yes"}\n');
  const [code] = await process.exited;
  assert.equal(code, 1);
  assert.equal(readJson(audio).devices.speaker.muted, false);
});

test('killing supervisor and blocked worker leaves durable recovery for a new process', async () => {
  const { directory, audio } = setupFiles();
  const process = rawHelper(directory, audio);
  await until(() => process.messages.some((m) => m.type === 'ready'));
  process.lease(true);
  await until(() => readJson(audio).devices.speaker.muted);
  const blocked = audio.replace(/\.json$/, '.blocked');
  const state = readJson(audio);
  state.blockOperation = 'get';
  writeJson(audio, state);
  await until(() => existsSync(blocked));
  const workerPid = Number(readFileSync(blocked, 'utf8'));
  assert.equal(workerPid, process.messages.find((m) => m.type === 'ready').workerPid);
  process.child.kill();
  globalThis.process.kill(workerPid);
  await process.exited;
  assert.equal(readJson(audio).devices.speaker.muted, true);
  state.blockOperation = null;
  writeJson(audio, state);
  const recovery = rawHelper(directory, audio, ['--recover-only']);
  const [code] = await recovery.exited;
  assert.equal(code, 0);
  assert.equal(readJson(audio).devices.speaker.muted, false);
  assert.equal(readJson(audio).devices['already-muted'].muted, true);
  assert.deepEqual(readJson(join(directory, 'recovery.json')).restoreUnmuted, []);
});

test('pending unplugged-device restoration is retained and not reported successful', async () => {
  const { directory, audio } = setupFiles();
  const helper = new HelperClient({
    dataDir: directory, maxMuteSeconds: 10, desired: () => true, changed() {},
    extraArgs: ['--simulation-file', audio], expectedBackend: 'simulation',
  });
  await helper.start();
  await until(() => readJson(audio).devices.speaker.muted);
  const state = readJson(audio);
  state.devices.speaker.active = false;
  writeJson(audio, state);
  assert.equal(await helper.stop(), false);
  assert.deepEqual(readJson(join(directory, 'recovery.json')).restoreUnmuted, ['speaker']);
  state.devices.speaker.active = true;
  writeJson(audio, state);
  const recovery = rawHelper(directory, audio, ['--recover-only']);
  const [code] = await recovery.exited;
  assert.equal(code, 0);
  assert.equal(readJson(audio).devices.speaker.muted, false);
});

test('corrupt recovery journal refuses startup without changing audio', async () => {
  const { directory, audio } = setupFiles();
  const before = readFileSync(audio, 'utf8');
  writeFileSync(join(directory, 'recovery.json'), '{broken');
  const process = rawHelper(directory, audio);
  const [code] = await process.exited;
  assert.equal(code, 1);
  assert.equal(readFileSync(audio, 'utf8'), before);
  assert.ok(process.messages.some((m) => m.type === 'error'));
});

test('prewarming the maximum supported topology does not lose the startup handshake', async () => {
  const { directory, audio } = setupFiles();
  const devices = Object.fromEntries(Array.from({ length: 128 }, (_, n) =>
    [`premuted-${n}`, { active: true, muted: true }]));
  writeJson(audio, { devices });
  const helper = new HelperClient({
    dataDir: directory, maxMuteSeconds: 10, desired: () => true, changed() {},
    extraArgs: ['--simulation-file', audio], expectedBackend: 'simulation',
  });
  try {
    await helper.start();
    await until(() => helper.status.active && helper.status.prepared);
    assert.equal(helper.error, null);
    assert.equal(helper.status.owned, 0);
    assert.ok(Object.values(readJson(audio).devices).every((device: any) => device.muted));
  } finally { assert.equal(await helper.stop(), true); }
});

test('a second helper cannot take over an active recovery directory', async () => {
  const { directory, audio } = setupFiles();
  const first = rawHelper(directory, audio);
  try {
    await until(() => first.messages.some((m) => m.type === 'ready'));
    first.lease(true);
    await until(() => readJson(audio).devices.speaker.muted);
    const second = rawHelper(directory, audio, ['--recover-only']);
    const [code] = await second.exited;
    assert.equal(code, 1);
    assert.equal(readJson(audio).devices.speaker.muted, true);
  } finally { await cleanup(first.child); }
  assert.equal(readJson(audio).devices.speaker.muted, false);
});

test('the production client rejects an accidentally configured simulated backend', async () => {
  const { directory, audio } = setupFiles();
  const helper = new HelperClient({
    dataDir: directory, maxMuteSeconds: 10, desired: () => true, changed() {},
    extraArgs: ['--simulation-file', audio],
  });
  await assert.rejects(helper.start(), /mismatch/);
  assert.equal(readJson(audio).devices.speaker.muted, false);
});

test('killing the Node host restores audio in the independent helper', async () => {
  const { directory, audio } = setupFiles();
  const host = spawn(process.execPath, [
    join(ROOT, 'tests', 'integration', 'host-fixture.mjs'), directory, audio,
  ], { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, shell: false });
  host.stdout.resume();
  host.stderr.resume();
  const exited = once(host, 'close');
  try {
    await until(() => readJson(audio).devices.speaker.muted);
    host.kill();
    await exited;
    await until(() => !readJson(audio).devices.speaker.muted);
    await until(() => readJson(join(directory, 'recovery.json')).restoreUnmuted.length === 0);
    assert.deepEqual(readJson(join(directory, 'recovery.json')).restoreUnmuted, []);
  } finally {
    if (host.exitCode === null && host.signalCode === null) { host.kill(); await exited; }
  }
});

test('always-listening snapshot never sends a mute to the native helper', async () => {
  const { directory, audio } = setupFiles();
  const server = await voiceServer();
  const snapshot = state(true);
  snapshot.input.mode = 'always';
  server.snapshot(snapshot);
  let helper: HelperClient;
  const bridge = new VoiceBridge({ origin: server.origin, password: () => '', changed: () => helper?.changed() });
  helper = new HelperClient({
    dataDir: directory, maxMuteSeconds: 10, desired: () => bridge.policy.active(performance.now()),
    changed() {}, extraArgs: ['--simulation-file', audio], expectedBackend: 'simulation',
  });
  try {
    await helper.start();
    bridge.start();
    await until(() => bridge.policy.snapshot !== null);
    server.send({ type: 'level', open: true });
    await new Promise((resolve) => setTimeout(resolve, 150));
    assert.equal(readJson(audio).devices.speaker.muted, false);
  } finally { bridge.stop(); await helper.stop(); await server.close(); }
});

for (const operation of ['get', 'restore']) {
  test(`an independently supervised blocked ${operation} call is terminated and recovered`, { timeout: 15_000 }, async () => {
    const { directory, audio } = setupFiles();
    const blocked = audio.replace(/\.json$/, '.blocked');
    let desired = true;
    const helper = new HelperClient({
      dataDir: directory, maxMuteSeconds: 10, desired: () => desired, changed() {},
      extraArgs: ['--simulation-file', audio], expectedBackend: 'simulation',
    });
    try {
      await helper.start();
      await until(() => readJson(audio).devices.speaker.muted);
      const state = readJson(audio);
      state.blockOperation = operation;
      writeJson(audio, state);
      if (operation === 'restore') {
        desired = false;
        helper.changed();
      }
      await until(() => existsSync(blocked));
      state.blockOperation = null;
      writeJson(audio, state);
      await until(() => helper.error !== null, 4_000);
      assert.match(helper.error!, /unresponsive/);
      await until(() => !readJson(audio).devices.speaker.muted, 4_000);
      await until(() => readJson(join(directory, 'recovery.json')).restoreUnmuted.length === 0);
      assert.equal(helper.ready, false);
      assert.equal(readJson(audio).devices['already-muted'].muted, true);
    } finally { await helper.stop(); }
  });
}

test('a persistently blocked recovery exits with a retained journal, not an endless worker loop', { timeout: 18_000 }, async () => {
  const { directory, audio } = setupFiles();
  const helper = new HelperClient({
    dataDir: directory, maxMuteSeconds: 10, desired: () => true, changed() {},
    extraArgs: ['--simulation-file', audio], expectedBackend: 'simulation',
  });
  try {
    await helper.start();
    await until(() => readJson(audio).devices.speaker.muted);
    const state = readJson(audio);
    state.blockOperation = 'get';
    writeJson(audio, state);
    await until(() => helper.error !== null, 4_000);
    assert.equal(await helper.stop(), false);
    assert.equal(readJson(audio).devices.speaker.muted, true);
    assert.deepEqual(readJson(join(directory, 'recovery.json')).restoreUnmuted, ['speaker']);
    state.blockOperation = null;
    writeJson(audio, state);
    const recovery = rawHelper(directory, audio, ['--recover-only']);
    const [code] = await recovery.exited;
    assert.equal(code, 0);
    assert.equal(readJson(audio).devices.speaker.muted, false);
  } finally { await helper.stop(); }
});

test('progress across slow devices is not mistaken for one blocked audio call', { timeout: 15_000 }, async () => {
  const { directory, audio } = setupFiles();
  writeJson(audio, {
    devices: {
      one: { active: true, muted: false }, two: { active: true, muted: false },
      three: { active: true, muted: false },
    },
    muteDelayMs: 780,
  });
  const helper = new HelperClient({
    dataDir: directory, maxMuteSeconds: 10, desired: () => true, changed() {},
    extraArgs: ['--simulation-file', audio], expectedBackend: 'simulation',
  });
  try {
    await helper.start();
    await until(() => helper.status.active && helper.status.owned === 3, 6000);
    assert.equal(helper.error, null);
    assert.ok(Object.values(readJson(audio).devices).every((device: any) => device.muted));
    assert.ok(helper.timings.filter((row) => row.phase === 'audio.mute').length >= 3);
  } finally { assert.equal(await helper.stop(), true); }
});

test('a stalled mute reports its operation phase and never loses the pending recovery record', { timeout: 12_000 }, async () => {
  const { directory, audio } = setupFiles();
  const state = readJson(audio);
  state.blockOperation = 'mute';
  writeJson(audio, state);
  const helper = new HelperClient({
    dataDir: directory, maxMuteSeconds: 10, desired: () => true, changed() {},
    extraArgs: ['--simulation-file', audio], expectedBackend: 'simulation',
  });
  try {
    await helper.start();
    await until(() => helper.error !== null, 5000);
    assert.match(helper.error!, /unresponsive during audio\.mute/);
    await helper.stop();
    assert.equal(readJson(audio).devices.speaker.muted, false);
    assert.deepEqual(readJson(join(directory, 'recovery.json')).restoreUnmuted, []);
  } finally { await helper.stop(); }
});
