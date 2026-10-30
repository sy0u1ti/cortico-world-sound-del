import { test } from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';
import { VoiceBridge } from '../src/voice-bridge.js';
import { until, voiceServer, state } from './fixtures.js';

test('a stock host arms direct detection with its own shortcut, without any registration or host patch', async () => {
  const server = await voiceServer();
  const bridge = new VoiceBridge({
    origin: server.origin, password: () => '', changed() {},
  });
  try {
    bridge.start();
    await until(() => bridge.state === 'online');
    assert.deepEqual(bridge.policy.directHotkey(performance.now()), {
      keys: [0xa4], taps: 2, mode: 'hold',
    });
    assert.equal(bridge.policy.active(performance.now()), false);
    assert.ok(server.requests.every((request) => request.url.startsWith('/api/webui/')));
    assert.equal(server.wss.clients.size, 1);
  } finally { bridge.stop(); await server.close(); }
});

test('real local HTTP/WebSocket transports drive the gate and stop cleanly', async () => {
  const server = await voiceServer();
  const bridge = new VoiceBridge({ origin: server.origin, password: () => '', changed() {} });
  try {
    bridge.start();
    await until(() => bridge.policy.snapshot !== null);
    server.gate(true);
    await until(() => bridge.policy.active(performance.now()));
    server.gate(false);
    await until(() => !bridge.policy.active(performance.now()));
    assert.ok(server.requests.every((r) => r.url.startsWith('/api/webui/')));
  } finally { bridge.stop(); await server.close(); }
  assert.equal(bridge.policy.connected, false);
});

test('authentication uses the normal login route and session cookie', async () => {
  const server = await voiceServer({ password: 'test password' });
  const bridge = new VoiceBridge({ origin: server.origin, password: () => 'test password', changed() {} });
  try {
    bridge.start();
    await until(() => bridge.policy.snapshot !== null);
    server.gate(true);
    await until(() => bridge.policy.active(performance.now()));
    assert.ok(server.requests.some((r) => r.url === '/api/auth/login'));
    assert.ok(server.requests.some((r) => r.cookie === 'cortico_session=test_session'));
    assert.doesNotMatch(bridge.detail, /test password|test_session/);
  } finally { bridge.stop(); await server.close(); }
});

test('missing password is visible and never activates muting', async () => {
  const server = await voiceServer({ password: 'secret' });
  const bridge = new VoiceBridge({ origin: server.origin, password: () => '', changed() {} });
  try {
    bridge.start();
    await until(() => bridge.detail.includes('password required'));
    assert.equal(bridge.policy.active(performance.now()), false);
    assert.equal(server.wss.clients.size, 0);
  } finally { bridge.stop(); await server.close(); }
});

test('HTTP failure restores policy while the WebSocket remains available', async () => {
  const server = await voiceServer();
  const bridge = new VoiceBridge({ origin: server.origin, password: () => '', changed() {} });
  try {
    bridge.start();
    await until(() => bridge.policy.snapshot !== null);
    server.gate(true);
    await until(() => bridge.policy.active(performance.now()));
    server.availability(false);
    await until(() => !bridge.policy.connected);
    assert.equal(bridge.policy.active(performance.now()), false);
  } finally { bridge.stop(); await server.close(); }
});

test('disconnect invalidates the current gate and reconnects to a fresh state', async () => {
  const server = await voiceServer();
  const bridge = new VoiceBridge({ origin: server.origin, password: () => '', changed() {} });
  try {
    bridge.start();
    await until(() => bridge.policy.snapshot !== null);
    server.gate(true);
    await until(() => bridge.policy.active(performance.now()));
    for (const client of server.wss.clients) client.terminate();
    await until(() => !bridge.policy.connected);
    assert.equal(bridge.policy.active(performance.now()), false);
    server.snapshot(state(false));
    await until(() => bridge.policy.snapshot !== null);
    assert.equal(bridge.policy.active(performance.now()), false);
  } finally { bridge.stop(); await server.close(); }
});

test('stop during a delayed request does not create a late connection', async () => {
  const server = await voiceServer({ delayMs: 150 });
  const bridge = new VoiceBridge({ origin: server.origin, password: () => '', changed() {} });
  try {
    bridge.start();
    bridge.stop();
    await delay(250);
    assert.equal(server.wss.clients.size, 0);
    assert.equal(bridge.policy.connected, false);
  } finally { bridge.stop(); await server.close(); }
});

test('unexpected schema fails open and never retains the raw response', async () => {
  const server = await voiceServer();
  server.snapshot({ rawRecording: true, recent: [{ text: 'PRIVATE' }] });
  const bridge = new VoiceBridge({ origin: server.origin, password: () => '', changed() {} });
  try {
    bridge.start();
    await until(() => bridge.detail.includes('schema'));
    assert.equal(bridge.policy.active(performance.now()), false);
    assert.doesNotMatch(bridge.detail, /PRIVATE/);
  } finally { bridge.stop(); await server.close(); }
});
