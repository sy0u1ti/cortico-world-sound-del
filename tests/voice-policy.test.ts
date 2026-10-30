import { test } from 'node:test';
import assert from 'node:assert/strict';
import { VoicePolicy, parseVoiceSnapshot, STATE_FRESH_MS, MIC_OPEN_GRACE_MS } from '../src/voice-policy.js';
import { localOrigin, SOUND_DEL_DEFAULTS, validateConfig } from '../src/config.js';

export function voiceState(overrides: Record<string, unknown> = {}) {
  return {
    enabled: true,
    server: { phase: 'running' },
    mic: { state: 'on' },
    input: { mode: 'hold', open: false, openOnPress: false, hotkeyProblem: null },
    ...overrides,
  };
}

function setup(state = voiceState()) {
  const policy = new VoicePolicy();
  policy.connect();
  policy.accept(parseVoiceSnapshot(state), 0, policy.revision, 0);
  return policy;
}

test('open/close and cancellation restore the inactive policy', () => {
  const policy = setup();
  assert.equal(policy.active(0), false);
  policy.frame({ type: 'gate', open: true }, 10);
  assert.equal(policy.active(10), true);
  policy.frame({ type: 'gate', open: false }, 20);
  assert.equal(policy.active(20), false);
});

test('a gate without a verified snapshot never activates muting', () => {
  const policy = new VoicePolicy();
  policy.connect();
  policy.frame({ type: 'gate', open: true }, 0);
  assert.equal(policy.active(0), false);
});

test('always mode, disabled voice, backend errors and denied microphone fail open', () => {
  for (const overrides of [
    { enabled: false },
    { server: null },
    { server: { phase: 'error' } },
    { mic: { state: 'denied' } },
    { mic: { state: 'error' } },
    { input: { mode: 'always', open: true, openOnPress: false, hotkeyProblem: null } },
    { input: { mode: 'hold', open: true, openOnPress: false, hotkeyProblem: 'unavailable' } },
  ]) {
    const policy = setup(voiceState(overrides));
    policy.frame({ type: 'gate', open: true }, 10);
    assert.equal(policy.active(10), false);
  }
});

test('toggle mode remains active until the gate closes', () => {
  const policy = setup(voiceState({ input: { mode: 'toggle', open: true, openOnPress: false, hotkeyProblem: null } }));
  assert.equal(policy.active(0), true);
  policy.frame({ type: 'gate', open: false }, 10);
  assert.equal(policy.active(10), false);
});

test('stale state expires even if gate frames keep arriving', () => {
  const policy = setup();
  policy.frame({ type: 'level', open: true }, STATE_FRESH_MS + 1);
  assert.equal(policy.active(STATE_FRESH_MS + 1), false);
});

test('disconnect invalidates state and reconnect needs a new snapshot', () => {
  const policy = setup();
  policy.frame({ type: 'gate', open: true }, 0);
  policy.disconnect();
  assert.equal(policy.active(10), false);
  policy.connect();
  policy.frame({ type: 'gate', open: true }, 20);
  assert.equal(policy.active(20), false);
});

test('delayed HTTP state cannot undo a newer cancel frame', () => {
  const policy = setup();
  policy.frame({ type: 'gate', open: true }, 5);
  const revision = policy.revision;
  policy.frame({ type: 'gate', open: false }, 15);
  policy.accept(parseVoiceSnapshot(voiceState({
    input: { mode: 'hold', open: true, openOnPress: false, hotkeyProblem: null },
  })), 10, revision, 20);
  assert.equal(policy.active(20), false);
});

test('mic-on-press has a bounded opening grace, not a permanent bypass', () => {
  const policy = setup(voiceState({
    mic: { state: 'off' },
    input: { mode: 'hold', open: false, openOnPress: true, hotkeyProblem: null },
  }));
  policy.frame({ type: 'gate', open: true }, 0);
  assert.equal(policy.active(10), true);
  assert.equal(policy.active(MIC_OPEN_GRACE_MS + 1), false);
  policy.frame({ type: 'level', open: true }, MIC_OPEN_GRACE_MS + 10);
  assert.equal(policy.active(MIC_OPEN_GRACE_MS + 10), false);
});

test('transcripts and API configuration are not retained', () => {
  const snapshot = parseVoiceSnapshot(voiceState({ recent: [{ text: 'PRIVATE TRANSCRIPT' }], api: { key: 'SECRET' } }));
  assert.doesNotMatch(JSON.stringify(snapshot), /PRIVATE|SECRET|recent|api/);
  const policy = setup();
  policy.frame({ type: 'text', text: 'PRIVATE TRANSCRIPT' }, 10);
  assert.doesNotMatch(JSON.stringify(policy), /PRIVATE/);
});

test('unknown schemas are rejected rather than guessed', () => {
  assert.throws(() => parseVoiceSnapshot({ enabled: true, recording: true }));
  assert.throws(() => parseVoiceSnapshot(voiceState({ input: { mode: 'hold', open: 'yes' } })));
  const policy = setup();
  assert.throws(() => policy.frame({ type: 'gate', open: 1 }, 0));
});

test('only literal loopback origins are accepted, without credentials or paths', () => {
  assert.equal(localOrigin('http://localhost:17788/'), 'http://127.0.0.1:17788');
  assert.equal(localOrigin('http://[::1]:17788'), 'http://[::1]:17788');
  for (const url of ['https://example.com', 'http://127.0.0.1@evil.test', 'http://127.0.0.1/a', 'http://127.0.0.1/?token=x', 'file:///x']) {
    assert.throws(() => localOrigin(url));
  }
  assert.throws(() => validateConfig({ ...SOUND_DEL_DEFAULTS, maxMuteSeconds: 0 }));
  assert.throws(() => validateConfig({ ...SOUND_DEL_DEFAULTS, maxMuteSeconds: Infinity }));
});
