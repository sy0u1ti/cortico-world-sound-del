import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { HELPER_FILE, HelperClient } from '../../src/helper-client.js';
import { VoiceBridge } from '../../src/voice-bridge.js';
import { readJson, scratch, until, writeJson, voiceServer } from '../fixtures.js';
import type { DirectHotkey } from '../../src/hotkey.js';

const defaultKey: DirectHotkey = { keys: [164], taps: 1, mode: 'hold' };

async function raw() {
  const directory = scratch('direct-'), audio = join(directory, 'audio.json');
  writeJson(audio, { devices: {
    speaker: { muted: false, active: true }, premuted: { muted: true, active: true },
  } });
  const child = spawn(HELPER_FILE, ['--data-dir', directory, '--simulation-file', audio], {
    stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, shell: false,
  });
  const exit = once(child, 'close'), frames: any[] = [];
  let buffer = '';
  child.stdout.on('data', (chunk) => {
    buffer += String(chunk);
    let end;
    while ((end = buffer.indexOf('\n')) >= 0) {
      frames.push(JSON.parse(buffer.slice(0, end)));
      buffer = buffer.slice(end + 1);
    }
  });
  child.stdin.on('error', () => {});
  child.stderr.resume();
  const send = (message: object) => child.stdin.write(JSON.stringify(message) + '\n');
  const configure = (active: boolean, hotkey: DirectHotkey | null = defaultKey) =>
    send({ type: 'lease', active, captureId: active ? 'recording' : null, hotkey });
  const key = (down: boolean) => send({ type: 'simulationKey', down });
  const muted = () => readJson(audio).devices.speaker.muted as boolean;
  await until(() => frames.some((frame) => frame.type === 'ready'));
  configure(false);
  await until(() => frames.some((frame) => frame.type === 'status' && frame.directHotkey === true));
  return {
    directory, audio, child, frames, configure, key, muted,
    async close() {
      child.stdin.end();
      const timer = setTimeout(() => child.kill(), 8000);
      await exit;
      clearTimeout(timer);
      assert.equal(muted(), false);
      assert.equal(readJson(audio).devices.premuted.muted, true);
    },
  };
}

test('the native key path mutes while the host still reports a closed recording gate', async () => {
  const helper = await raw();
  try {
    helper.key(true);
    await until(helper.muted);
    await until(() => helper.frames.some((frame) => frame.captureId === 'hotkey-1'));
    assert.ok(helper.frames.some((frame) => frame.captureId === 'hotkey-1'));
    helper.configure(false);
    await delay(100);
    assert.equal(helper.muted(), true);
    helper.configure(true);
    helper.key(false);
    await delay(60);
    assert.equal(helper.muted(), true);
    helper.configure(false);
    await until(() => !helper.muted());
  } finally { await helper.close(); }
});

test('default double tap starts at the held press, without suppressing or synthesizing a system key', async () => {
  const helper = await raw();
  try {
    helper.configure(false, { ...defaultKey, taps: 2 });
    await delay(40);
    helper.key(true);
    await delay(100);
    assert.equal(helper.muted(), false);
    helper.key(false);
    await delay(50);
    helper.key(true);
    await until(helper.muted);
    helper.configure(true, { ...defaultKey, taps: 2 });
    helper.key(false);
    helper.configure(false, { ...defaultKey, taps: 2 });
    await until(() => !helper.muted());
  } finally { await helper.close(); }
});

test('batched rapid double-tap edges are not collapsed into a single held sample', async () => {
  const helper = await raw();
  try {
    helper.configure(false, { ...defaultKey, taps: 2 });
    helper.child.stdin.write([true, false, true].map(down =>
      JSON.stringify({ type: 'simulationKey', down })).join('\n') + '\n');
    await until(helper.muted, 1000);
    await until(() => helper.frames.some(f => f.type === 'latency'));
    assert.equal(helper.frames.find(f => f.type === 'latency').value.source, 'simulation');
  } finally { await helper.close(); }
});

test('latency ends at simulated mute readback, not when the request reaches the worker', async () => {
  const helper = await raw();
  try {
    const state = readJson(helper.audio);
    state.muteDelayMs = 80;
    writeJson(helper.audio, state);
    helper.key(true);
    await until(() => helper.frames.some(f => f.type === 'latency'));
    const value = helper.frames.find(f => f.type === 'latency').value;
    assert.equal(helper.muted(), true);
    assert.ok(value.firstMuteMs >= value.dispatchMs + 75);
    assert.ok(value.allMutedMs >= value.firstMuteMs);
    assert.equal(value.source, 'simulation');
  } finally { await helper.close(); }
});

test('already-muted devices do not generate a fictitious first-mute measurement', async () => {
  const helper = await raw();
  try {
    const state = readJson(helper.audio);
    state.devices.speaker.muted = true;
    writeJson(helper.audio, state);
    helper.key(true);
    await until(() => helper.frames.some(f => f.type === 'latency'));
    const value = helper.frames.find(f => f.type === 'latency').value;
    assert.equal(value.firstMuteMs, null);
    assert.ok(value.allMutedMs >= value.dispatchMs);
    helper.configure(true);
    helper.configure(false);
    await until(() => helper.frames.some(f => f.type === 'status' && f.captureId === null && !f.active));
    // This test owns the simulated manual change; restore it only after the helper has closed.
    helper.child.stdin.end();
    await once(helper.child, 'close');
    assert.equal(helper.muted(), true);
    state.devices.speaker.muted = false;
    writeJson(helper.audio, state);
  } finally { await helper.close(); }
});

test('an unconfirmed toggle gesture cannot mute indefinitely', async () => {
  const helper = await raw();
  const toggle = { ...defaultKey, mode: 'toggle' as const };
  try {
    helper.configure(false, toggle);
    await delay(50);
    helper.key(true);
    await until(helper.muted);
    await until(() => !helper.muted(), 2000);
    helper.configure(false, toggle);
    await delay(150);
    assert.equal(helper.muted(), false);
  } finally { await helper.close(); }
});

test('a held physical shortcut is not revoked just because the start notification takes longer than 1.2 seconds', async () => {
  const helper = await raw();
  let lease: ReturnType<typeof setInterval> | undefined;
  try {
    lease = setInterval(() => helper.configure(false), 400);
    helper.key(true);
    await until(helper.muted);
    await delay(1700);
    assert.equal(helper.muted(), true);
    clearInterval(lease);
    helper.configure(true);
    helper.key(false);
    helper.configure(false);
    await until(() => !helper.muted());
  } finally { clearInterval(lease); await helper.close(); }
});

test('a new press starts another direct mute even when an old host closed message is missing', async () => {
  const helper = await raw();
  try {
    helper.key(true);
    await until(helper.muted);
    helper.configure(true);
    await delay(60);
    helper.key(false);
    await until(() => !helper.muted());
    helper.configure(true);
    helper.key(true);
    await until(helper.muted);
    helper.configure(true);
    helper.configure(false);
    await until(() => !helper.muted(), 1000);
  } finally { await helper.close(); }
});

test('host cancellation restores even while the physical shortcut remains held', async () => {
  const helper = await raw();
  try {
    helper.key(true);
    await until(helper.muted);
    helper.configure(true);
    await delay(60);
    helper.configure(false);
    await until(() => !helper.muted());
    await delay(120);
    assert.equal(helper.muted(), false);
  } finally { await helper.close(); }
});

test('disabling direct eligibility cancels speculative muting immediately', async () => {
  const helper = await raw();
  try {
    helper.key(true);
    await until(helper.muted);
    helper.configure(false, null);
    await until(() => !helper.muted());
    helper.key(false);
    helper.key(true);
    await delay(80);
    assert.equal(helper.muted(), false);
  } finally { await helper.close(); }
});

test('toggle key-up keeps mute until the host ends the recording', async () => {
  const helper = await raw();
  const toggle = { ...defaultKey, mode: 'toggle' as const };
  try {
    helper.configure(false, toggle);
    await delay(40);
    helper.key(true);
    await until(helper.muted);
    helper.configure(true, toggle);
    helper.key(false);
    await delay(450);
    assert.equal(helper.muted(), true);
    helper.key(true);
    await delay(50);
    helper.configure(false, toggle);
    await until(() => !helper.muted());
  } finally { await helper.close(); }
});

test('the direct listener loses authorization if host leases stop', async () => {
  const helper = await raw();
  try {
    helper.key(true);
    await until(helper.muted);
    helper.configure(true);
    await until(() => !helper.muted(), 3500);
    assert.ok(helper.frames.some((frame) => frame.type === 'status' && !frame.active && !frame.directHotkey));
  } finally { await helper.close(); }
});

test('stock voice metadata reaches the native listener and HTTP failure disarms it', async () => {
  const directory = scratch('direct-bridge-'), audio = join(directory, 'audio.json');
  writeJson(audio, { devices: { speaker: { muted: false, active: true } } });
  const server = await voiceServer();
  let helper: HelperClient;
  const bridge = new VoiceBridge({ origin: server.origin, password: () => '', changed: () => helper?.changed() });
  helper = new HelperClient({
    dataDir: directory, maxMuteSeconds: 10, expectedBackend: 'simulation',
    extraArgs: ['--simulation-file', audio], changed() {},
    desired: () => bridge.policy.active(performance.now()),
    hotkey: () => bridge.policy.directHotkey(performance.now()),
  });
  try {
    await helper.start();
    bridge.start();
    await until(() => helper.status.directHotkey);
    server.availability(false);
    await until(() => !helper.status.directHotkey);
    assert.equal(readJson(audio).devices.speaker.muted, false);
  } finally { bridge.stop(); await helper.stop(); await server.close(); }
});
