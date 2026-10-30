import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseHotkey } from '../src/hotkey.js';
import { VoicePolicy, parseVoiceSnapshot, STATE_FRESH_MS } from '../src/voice-policy.js';
import { state } from './fixtures.js';
import { soundDelConfigGroup } from '../src/config.js';

test('parses the configured stock shortcuts without changing keys or tap count', () => {
  assert.deepEqual(parseHotkey('LeftAlt*2', 'hold'), { keys: [164], taps: 2, mode: 'hold' });
  assert.deepEqual(parseHotkey(' Ctrl + Space * 3 ', 'toggle'), { keys: [17, 32], taps: 3, mode: 'toggle' });
  assert.deepEqual(parseHotkey('F24', 'hold'), { keys: [135], taps: 1, mode: 'hold' });
  assert.deepEqual(parseHotkey('Mouse4', 'hold'), { keys: [5], taps: 1, mode: 'hold' });
  assert.deepEqual(parseHotkey('Ctrl+Ctrl+A', 'hold'), { keys: [17, 65], taps: 1, mode: 'hold' });
});

test('unknown, excessive or invalid shortcuts never become a key monitor', () => {
  for (const value of [null, 123, '', 'Escape', 'LeftAlt*4', 'F0', 'F25', '__proto__', 'constructor',
    'Ctrl+Alt+Shift+Win+A+B+C+D+E', 'x'.repeat(200)]) {
    assert.equal(parseHotkey(value, 'hold'), null);
  }
});

test('a closed recording gate can arm direct detection, but an ineligible or stale voice state cannot', () => {
  const policy = new VoicePolicy();
  policy.connect();
  policy.accept(parseVoiceSnapshot(state()), 0, policy.revision, 0);
  assert.equal(policy.active(0), false);
  assert.deepEqual(policy.directHotkey(0), { keys: [164], taps: 2, mode: 'hold' });
  assert.equal(policy.directHotkey(STATE_FRESH_MS + 1), null);
  for (const value of [
    { ...state(), enabled: false },
    { ...state(), server: { phase: 'starting' } },
    { ...state(), mic: { state: 'denied' } },
    { ...state(), mic: { state: 'off' } },
    { ...state(), input: { ...state().input, mode: 'always' } },
    { ...state(), input: { ...state().input, hotkeyProblem: 'Unavailable' } },
    { ...state(), captureGuard: { protocol: 1, required: true } },
  ]) {
    policy.accept(parseVoiceSnapshot(value), 0, policy.revision, 0);
    assert.equal(policy.directHotkey(0), null);
  }
});

test('opening the microphone on press does not require waiting for its on notification', () => {
  const policy = new VoicePolicy();
  policy.connect();
  policy.accept(parseVoiceSnapshot({
    ...state(), mic: { state: 'off' }, input: { ...state().input, openOnPress: true },
  }), 0, policy.revision, 0);
  assert.ok(policy.directHotkey(0));
  policy.disconnect();
  assert.equal(policy.directHotkey(0), null);
});

test('normal settings do not expose internal local transport or obsolete preparation settings', () => {
  const config = soundDelConfigGroup('zh').schema as { properties: Record<string, unknown> };
  assert.ok(config.properties['worlds.sound-del.maxMuteSeconds']);
  assert.equal(config.properties['worlds.sound-del.webUiUrl'], undefined);
  assert.equal(config.properties['worlds.sound-del.settleMs'], undefined);
});
