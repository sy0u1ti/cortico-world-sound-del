import { parseHotkey, type DirectHotkey } from './hotkey.js';

export const STATE_FRESH_MS = 2_000;
export const MIC_OPEN_GRACE_MS = 1_200;

export interface VoiceSnapshot {
  enabled: boolean;
  mode: 'hold' | 'toggle' | 'always';
  open: boolean;
  openOnPress: boolean;
  backend: string | null;
  mic: string;
  hotkeyProblem: boolean;
  captureGuard?: { protocol: number; required: boolean };
  hotkey: DirectHotkey | null;
}

const record = (value: unknown): Record<string, unknown> | null =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown> : null;

/** Only retain gate metadata. Transcripts, device names and API settings are discarded. */
export function parseVoiceSnapshot(value: unknown): VoiceSnapshot {
  const root = record(value), input = record(root?.input), mic = record(root?.mic);
  const server = record(root?.server);
  const guard = record(root?.captureGuard);
  if (!root || !input || !mic || typeof root.enabled !== 'boolean'
    || !['hold', 'toggle', 'always'].includes(String(input.mode))
    || typeof input.open !== 'boolean' || typeof input.openOnPress !== 'boolean'
    || typeof mic.state !== 'string'
    || !(root.server === null || (server && typeof server.phase === 'string'))
    || !(input.hotkeyProblem === null || typeof input.hotkeyProblem === 'string')) {
    throw new Error('Unsupported Coopanion voice-state schema; muting is disabled');
  }
  return {
    enabled: root.enabled,
    mode: input.mode as VoiceSnapshot['mode'],
    open: input.open,
    openOnPress: input.openOnPress,
    backend: server?.phase as string | undefined ?? null,
    mic: mic.state,
    hotkeyProblem: input.hotkeyProblem !== null,
    hotkey: input.mode === 'always' ? null : parseHotkey(input.hotkey, input.mode as 'hold' | 'toggle'),
    ...(guard && typeof guard.protocol === 'number' && typeof guard.required === 'boolean'
      ? { captureGuard: { protocol: guard.protocol, required: guard.required } } : {}),
  };
}

export class VoicePolicy {
  connected = false;
  snapshot: VoiceSnapshot | null = null;
  stateAt = -Infinity;
  revision = 0;
  private gate = false;
  private openedAt = -Infinity;

  connect(): void {
    this.disconnect();
    this.connected = true;
  }

  disconnect(): void {
    this.connected = false;
    this.snapshot = null;
    this.stateAt = -Infinity;
    this.gate = false;
    this.openedAt = -Infinity;
    this.revision++;
  }

  frame(value: unknown, now: number): void {
    const message = record(value);
    if (!this.connected || !message || !['gate', 'level'].includes(String(message.type))) return;
    if (typeof message.open !== 'boolean') {
      throw new Error('Unsupported Coopanion voice-frame schema; muting is disabled');
    }
    this.revision++;
    this.setGate(message.open, now);
  }

  accept(snapshot: VoiceSnapshot, requestedAt: number, requestRevision: number, now: number): void {
    if (!this.connected) return;
    this.snapshot = snapshot;
    this.stateAt = requestedAt;
    // A delayed HTTP response must never reopen a gate already closed by the stream.
    if (this.revision === requestRevision) this.setGate(snapshot.open, now);
  }

  active(now: number): boolean {
    const state = this.snapshot;
    if (!this.connected || !state || now - this.stateAt > STATE_FRESH_MS
      || !state.enabled || state.mode === 'always' || state.backend !== 'running'
      || state.hotkeyProblem || !this.gate) return false;
    if (state.captureGuard?.required) return false;
    return state.mic === 'on' || (state.mic === 'off' && state.mode === 'hold'
      && state.openOnPress && now - this.openedAt <= MIC_OPEN_GRACE_MS);
  }

  directHotkey(now: number): DirectHotkey | null {
    const state = this.snapshot;
    if (!this.connected || !state || now - this.stateAt > STATE_FRESH_MS
      || !state.enabled || state.mode === 'always' || state.backend !== 'running'
      || state.hotkeyProblem || state.captureGuard?.required) return null;
    if (state.mic !== 'on' && !(state.mic === 'off' && state.mode === 'hold' && state.openOnPress)) return null;
    return state.hotkey;
  }

  private setGate(open: boolean, now: number): void {
    if (open && !this.gate) this.openedAt = now;
    this.gate = open;
  }
}
