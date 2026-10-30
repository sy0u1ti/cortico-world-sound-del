import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import type { DirectHotkey } from './hotkey.js';
import { writeFile } from 'node:fs/promises';

export const HELPER_FILE = fileURLToPath(new URL('../bin/win32-x64/sound-del-helper.exe', import.meta.url));
export const LEASE_INTERVAL_MS = 400;
export const HELPER_PROGRESS_TIMEOUT_MS = 2_500;
export const HELPER_OUTPUT_LIMIT = 16_384;
const PROTOCOL = 6;

export interface MuteLatency {
  source: 'hook' | 'poll' | 'host' | 'simulation';
  dispatchMs: number;
  firstMuteMs: number | null;
  allMutedMs: number | null;
}

export interface HelperStatus {
  active: boolean;
  owned: number;
  pendingRestore: number;
  overrides: number;
  suspended: string | null;
  recoveryUncertain: boolean;
  captureId: string | null;
  prepared: boolean;
  directHotkey: boolean;
}

export interface HelperOptions {
  dataDir: string;
  maxMuteSeconds: number;
  desired(): boolean;
  captureId?(): string | null;
  hotkey?(): DirectHotkey | null;
  changed(): void;
  executable?: string;
  extraArgs?: string[];
  expectedBackend?: 'windows' | 'simulation';
}

export function defaultDataDir(): string {
  const local = process.env.LOCALAPPDATA;
  if (!local) throw new Error('LOCALAPPDATA is not set');
  return join(local, 'CoopanionSoundDel');
}

const count = (value: unknown): value is number =>
  typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 128;

export class HelperClient {
  status: HelperStatus = {
    active: false, owned: 0, pendingRestore: 0, overrides: 0, suspended: null, recoveryUncertain: true,
    captureId: null, prepared: false, directHotkey: false,
  };
  error: string | null = null;
  ready = false;
  readonly timings: Array<{ phase: string; ms: number; ok: boolean }> = [];
  readonly latencies: MuteLatency[] = [];
  private child: ChildProcessWithoutNullStreams | null = null;
  private heartbeat: ReturnType<typeof setInterval> | null = null;
  private exited: Promise<void> = Promise.resolve();
  private stopPromise: Promise<boolean> | null = null;
  private stopping = false;
  private failed = false;
  private lastLease = '';
  private verifiedStop = false;
  private exitCode: number | null = null;
  private handshakeReceived = false;
  private stoppedReceived = false;
  private lastProgressAt = 0;
  private lastSequence = -1;
  private discardOutput = () => {};
  private rejectStartup = (_error: Error) => {};
  private failureRecord: Promise<void> = Promise.resolve();
  private lastSlowRecordAt = -Infinity;
  private savingSlowRecord = false;
  private lastLatencyRecordAt = -Infinity;
  private savingLatencyRecord = false;

  constructor(private readonly options: HelperOptions) {}

  async start(): Promise<void> {
    if (this.child || this.stopping) throw new Error('Create a new audio client to restart');
    const child = spawn(this.options.executable ?? HELPER_FILE, [
      ...(this.options.extraArgs ?? []),
      '--data-dir', this.options.dataDir,
      '--max-mute-seconds', String(this.options.maxMuteSeconds),
    ], { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, detached: true, shell: false });
    this.child = child;
    child.unref();
    this.exited = new Promise((resolve) => {
      child.once('close', () => resolve());
      child.once('error', () => resolve());
    });
    child.stdin.on('error', () => { if (!this.stopping) this.fail('Audio helper input closed'); });
    child.stderr.resume();
    let resolveReady: () => void = () => {};
    const opened = new Promise<void>((resolve, reject) => {
      resolveReady = resolve;
      this.rejectStartup = reject;
    });
    const timer = setTimeout(() => this.fail('Audio helper startup timed out'), 8_000);
    let buffer: Buffer = Buffer.alloc(0);
    let discarded = false;
    const data = (chunk: Buffer) => {
      if (discarded) return;
      let cursor = 0, frames = 0;
      while (cursor < chunk.length) {
        const newline = chunk.indexOf(10, cursor);
        const end = newline < 0 ? chunk.length : newline;
        if (buffer.length + end - cursor > HELPER_OUTPUT_LIMIT || ++frames > 512) {
          this.fail('Invalid audio helper output: size limit exceeded', true);
          return;
        }
        const fragment = chunk.subarray(cursor, end);
        if (newline < 0) {
          // Copy the bounded remainder, never retain a slice of an arbitrarily large chunk.
          buffer = Buffer.concat([buffer, fragment]);
          return;
        }
        const line = (buffer.length ? Buffer.concat([buffer, fragment]) : fragment).toString('utf8');
        buffer = Buffer.alloc(0);
        try {
          const value: unknown = JSON.parse(line);
          if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid helper message');
          const message = value as Record<string, unknown>;
          this.message(message, resolveReady);
        } catch (error) {
          this.fail(error instanceof Error ? error.message : 'Invalid helper output', true);
          return;
        }
        cursor = end + 1;
      }
    };
    this.discardOutput = () => {
      discarded = true;
      buffer = Buffer.alloc(0);
      child.stdout.off('data', data);
      child.stdout.destroy();
    };
    child.stdout.on('data', data);
    child.stdout.on('error', () => this.fail('Audio helper output closed', true));
    child.once('error', () => this.fail('Cannot start audio helper; run npm run build:native'));
    child.once('exit', (code) => {
      this.exitCode = code;
      this.ready = false;
      if (this.heartbeat) clearInterval(this.heartbeat);
      this.heartbeat = null;
      if (!this.stopping) this.fail(`Audio helper exited (${code ?? 'terminated'}); restart this World to recover`);
      this.rejectStartup(new Error(this.error ?? 'Audio helper exited'));
      this.options.changed();
    });
    try {
      await opened;
      if (this.stopping || this.failed) throw new Error(this.error ?? 'Audio helper stopped during startup');
      this.sendLease();
      this.heartbeat = setInterval(() => {
        if (performance.now() - this.lastProgressAt > HELPER_PROGRESS_TIMEOUT_MS) {
          this.fail('Audio helper stopped reporting progress; restoration is unconfirmed');
          return;
        }
        this.sendLease();
      }, LEASE_INTERVAL_MS);
    } catch (error) {
      await this.stop();
      throw error;
    } finally {
      clearTimeout(timer);
    }
  }

  private message(message: Record<string, unknown>, resolveReady: () => void): void {
    switch (message.type) {
      case 'ready':
        if (this.handshakeReceived || message.protocol !== PROTOCOL
          || message.backend !== (this.options.expectedBackend ?? 'windows')) {
          throw new Error('Audio helper protocol or backend mismatch');
        }
        this.handshakeReceived = true;
        if (!this.stopping && !this.failed) {
          this.ready = true;
          this.lastProgressAt = performance.now();
          resolveReady();
        }
        break;
      case 'heartbeat':
        if (message.protocol !== PROTOCOL || typeof message.sequence !== 'number'
          || !Number.isSafeInteger(message.sequence) || message.sequence <= this.lastSequence) {
          throw new Error('Invalid audio helper progress sequence');
        }
        this.lastSequence = message.sequence;
        this.lastProgressAt = performance.now();
        break;
      case 'status':
        if (typeof message.active !== 'boolean' || !count(message.owned)
          || !count(message.pendingRestore) || !count(message.overrides)
          || typeof message.recoveryUncertain !== 'boolean'
          || typeof message.prepared !== 'boolean'
          || typeof message.directHotkey !== 'boolean'
          || !(message.captureId === null || (typeof message.captureId === 'string' && /^[A-Za-z0-9-]{1,96}$/.test(message.captureId)))
          || (message.prepared && (!message.active || message.captureId === null || message.recoveryUncertain))
          || !(message.suspended === null || (typeof message.suspended === 'string' && message.suspended.length <= 4096))) {
          throw new Error('Invalid helper status');
        }
        this.status = {
          active: message.active, owned: message.owned, pendingRestore: message.pendingRestore,
          overrides: message.overrides, suspended: message.suspended, recoveryUncertain: message.recoveryUncertain,
          captureId: message.captureId, prepared: message.prepared, directHotkey: message.directHotkey,
        };
        break;
      case 'error':
        this.fail(typeof message.message === 'string' ? message.message : 'Audio helper failed');
        break;
      case 'diagnostic': {
        if (typeof message.phase !== 'string' || !/^(audio\.(enumerate|read|mute|restore)|journal\.(load|save))$/.test(message.phase)
          || typeof message.ms !== 'number' || !Number.isFinite(message.ms) || message.ms < 0 || message.ms >= 60_000
          || typeof message.ok !== 'boolean') throw new Error('Invalid helper diagnostic');
        this.timings.push({ phase: message.phase, ms: message.ms, ok: message.ok });
        if (this.timings.length > 32) this.timings.shift();
        if (message.ms >= 100 && !this.savingSlowRecord && performance.now() - this.lastSlowRecordAt >= 5000) {
          this.lastSlowRecordAt = performance.now();
          this.savingSlowRecord = true;
          void this.saveDiagnostics('last-slow-operation.json').finally(() => { this.savingSlowRecord = false; });
        }
        break;
      }
      case 'latency': {
        const value = message.value as Partial<MuteLatency> | null;
        const ms = (n: unknown): n is number =>
          typeof n === 'number' && Number.isFinite(n) && n >= 0 && n < 60_000;
        if (!value || typeof value !== 'object' || Array.isArray(value)
          || !['hook', 'poll', 'host', 'simulation'].includes(value.source ?? '')
          || !ms(value.dispatchMs)
          || !(value.firstMuteMs === null || (ms(value.firstMuteMs) && value.firstMuteMs >= value.dispatchMs))
          || !(value.allMutedMs === null || (ms(value.allMutedMs) && value.allMutedMs >= (value.firstMuteMs ?? value.dispatchMs)))) {
          throw new Error('Invalid mute completion timing');
        }
        this.latencies.push({
          source: value.source!, dispatchMs: value.dispatchMs,
          firstMuteMs: value.firstMuteMs, allMutedMs: value.allMutedMs,
        });
        if (this.latencies.length > 32) this.latencies.shift();
        if (!this.savingLatencyRecord && performance.now() - this.lastLatencyRecordAt >= 5000) {
          this.lastLatencyRecordAt = performance.now();
          this.savingLatencyRecord = true;
          void this.saveDiagnostics('last-latency.json').finally(() => { this.savingLatencyRecord = false; });
        }
        break;
      }
      case 'stopped':
        if (this.stoppedReceived || typeof message.restored !== 'boolean' || !count(message.pendingRestore)
          || (message.restored && message.pendingRestore !== 0)) throw new Error('Invalid restoration acknowledgement');
        this.stoppedReceived = true;
        this.verifiedStop = this.handshakeReceived && message.restored && message.pendingRestore === 0;
        if (this.verifiedStop) {
          this.status = { ...this.status, active: false, prepared: false, directHotkey: false, captureId: null, owned: 0, pendingRestore: 0, recoveryUncertain: false };
        }
        break;
      default:
        throw new Error('Unknown audio helper message');
    }
    this.options.changed();
  }

  changed(): void {
    if (!this.ready || this.stopping) return;
    const lease = this.lease();
    if (lease !== this.lastLease) this.sendLease(lease);
  }

  private lease(): string {
    const active = this.options.desired();
    return JSON.stringify({
      type: 'lease', active, captureId: active ? (this.options.captureId?.() ?? 'recording') : null,
      hotkey: this.options.hotkey?.() ?? null,
    }) + '\n';
  }

  private sendLease(lease = this.lease()): void {
    if (!this.ready || this.stopping || this.failed) return;
    const input = this.child?.stdin;
    if (!input || input.destroyed || input.writableEnded) return;
    if (input.writableLength > 4_096) {
      this.fail('Audio helper is not consuming its lease');
      return;
    }
    input.write(lease);
    this.lastLease = lease;
  }

  private shutdown(): void {
    if (this.stopping) return;
    this.stopping = true;
    this.ready = false;
    if (this.heartbeat) clearInterval(this.heartbeat);
    this.heartbeat = null;
    const input = this.child?.stdin;
    if (input && !input.destroyed && !input.writableEnded) {
      if (input.writableLength > 4_096) input.destroy();
      else input.end('{"type":"shutdown"}\n');
    }
  }

  private fail(message: string, invalidOutput = false): void {
    const firstFailure = this.error === null;
    this.error ??= message.slice(0, 300);
    this.failed = true;
    if (invalidOutput) {
      this.verifiedStop = false;
      this.discardOutput();
    }
    this.status = { ...this.status, active: false, prepared: false, directHotkey: false, recoveryUncertain: !this.verifiedStop };
    this.shutdown();
    this.rejectStartup(new Error(this.error));
    this.options.changed();
    if (firstFailure) {
      this.failureRecord = this.saveDiagnostics('last-failure.json');
    }
  }

  private saveDiagnostics(file: string): Promise<void> {
    // Bounded, asynchronous diagnostics; never store voice content or device identifiers.
    return writeFile(join(this.options.dataDir, file), JSON.stringify({
      protocol: PROTOCOL, backend: this.options.expectedBackend ?? 'windows',
      at: new Date().toISOString(), error: this.error, timings: this.timings, latencies: this.latencies,
      active: this.status.active, owned: this.status.owned, pendingRestore: this.status.pendingRestore,
    }, null, 2)).catch(() => {});
  }

  stop(): Promise<boolean> {
    this.stopPromise ??= this.finishStop();
    return this.stopPromise;
  }

  private async finishStop(): Promise<boolean> {
    this.shutdown();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ended = await Promise.race([
      this.exited.then(() => true),
      new Promise<false>((resolve) => { timer = setTimeout(() => resolve(false), 8_000); }),
    ]);
    if (timer) clearTimeout(timer);
    this.discardOutput();
    this.child?.stderr.destroy();
    this.child?.stdin.destroy();
    return ended && this.exitCode !== null && this.verifiedStop;
  }
}
