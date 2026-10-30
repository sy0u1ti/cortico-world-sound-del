import WebSocket from 'ws';
import { localOrigin } from './config.js';
import { parseVoiceSnapshot, VoicePolicy } from './voice-policy.js';

const STATE_PATH = '/api/webui/providers/world%3Adesktop-pet/panels/voice/state';
const STREAM_PATH = '/ws/providers/world%3Adesktop-pet/panels/voice';
const REQUEST_TIMEOUT_MS = 1_200;
const BODY_LIMIT = 256 * 1024;

export interface VoiceBridgeOptions {
  origin: string;
  password(): string;
  changed(): void;
}

export class VoiceBridge {
  readonly policy = new VoicePolicy();
  detail = 'Connecting to Coopanion';
  state: 'connecting' | 'online' | 'error' | 'stopped' = 'stopped';
  private readonly origin: string;
  private running = false;
  private generation = 0;
  private cookie = '';
  private socket: WebSocket | null = null;
  private timer: ReturnType<typeof setInterval> | null = null;
  private retry: ReturnType<typeof setTimeout> | null = null;
  private readonly requests = new Set<AbortController>();
  private polling = false;
  private failures = 0;
  private pongAt = 0;

  constructor(private readonly options: VoiceBridgeOptions) {
    this.origin = localOrigin(options.origin);
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    void this.connect();
  }

  stop(): void {
    this.running = false;
    this.generation++;
    this.cleanup();
    this.policy.disconnect();
    this.detail = 'Stopped';
    this.state = 'stopped';
    this.options.changed();
  }

  private cleanup(): void {
    if (this.timer) clearInterval(this.timer);
    if (this.retry) clearTimeout(this.retry);
    this.timer = this.retry = null;
    for (const controller of this.requests) controller.abort();
    this.requests.clear();
    const socket = this.socket;
    this.socket = null;
    if (socket && socket.readyState !== WebSocket.CLOSED) socket.terminate();
    this.polling = false;
  }

  private async connect(): Promise<void> {
    const generation = ++this.generation;
    const current = () => this.running && generation === this.generation;
    this.detail = 'Connecting to Coopanion';
    this.state = 'connecting';
    try {
      // Establish normal HTTP authentication before the WebSocket handshake.
      await this.snapshot();
      if (!current()) return;
      const socket = new WebSocket(this.origin.replace(/^http:/, 'ws:') + STREAM_PATH, {
        headers: { Origin: this.origin, ...(this.cookie ? { Cookie: this.cookie } : {}) },
        handshakeTimeout: REQUEST_TIMEOUT_MS,
        maxPayload: BODY_LIMIT,
        followRedirects: false,
        perMessageDeflate: false,
      });
      this.socket = socket;
      socket.on('error', () => { if (current()) this.fail('Voice WebSocket failed'); });
      socket.on('close', () => { if (current()) this.fail('Voice WebSocket disconnected'); });
      socket.on('pong', () => { if (current()) this.pongAt = performance.now(); });
      socket.on('message', (bytes, binary) => {
        if (!current()) return;
        try {
          if (binary) throw new Error('Unexpected binary voice frame');
          this.policy.frame(JSON.parse(bytes.toString()), performance.now());
          this.options.changed();
        } catch {
          this.fail('Unsupported voice stream; muting is disabled');
        }
      });
      socket.on('open', () => {
        if (!current()) return;
        this.policy.connect();
        this.pongAt = performance.now();
        void this.poll(generation);
        this.timer = setInterval(() => {
          if (!current()) return;
          if (performance.now() - this.pongAt > 2_500) {
            this.fail('Voice connection timed out');
            return;
          }
          socket.ping();
          void this.poll(generation);
        }, 750);
      });
    } catch (error) {
      if (current()) this.fail(error instanceof Error ? error.message : 'Voice connection failed');
    }
  }

  private async poll(generation: number): Promise<void> {
    if (this.polling) return;
    this.polling = true;
    const revision = this.policy.revision, requestedAt = performance.now();
    try {
      const snapshot = await this.snapshot();
      if (!this.running || generation !== this.generation) return;
      this.policy.accept(snapshot, requestedAt, revision, performance.now());
      if (snapshot.captureGuard?.required) throw new Error('Disable host capture preparation to use direct shortcut mode');
      this.detail = snapshot.mode === 'always' ? 'Always-listening mode is excluded' : 'Connected';
      this.state = 'online';
      this.failures = 0;
      this.options.changed();
    } catch (error) {
      if (this.running && generation === this.generation) {
        this.fail(error instanceof Error ? error.message : 'Voice state unavailable');
      }
    } finally {
      if (generation === this.generation) this.polling = false;
    }
  }

  private fail(detail: string): void {
    if (!this.running) return;
    this.generation++;
    this.cleanup();
    this.policy.disconnect();
    this.detail = detail;
    this.state = 'error';
    this.options.changed();
    const delay = Math.min(30_000, 1_000 * 2 ** Math.min(this.failures++, 5));
    this.retry = setTimeout(() => { this.retry = null; void this.connect(); }, delay);
  }

  private async snapshot() {
    let response = await this.request(STATE_PATH);
    if (response.status === 401) {
      this.cookie = '';
      const password = this.options.password();
      if (!password) throw new Error('WebUI password required: set CORTICO_SOUND_DEL_WEB_PASSWORD or CORTICO_WEB_PASSWORD');
      const login = await this.request('/api/auth/login', password);
      if (login.status !== 200) throw new Error('WebUI authentication failed');
      const cookie = login.cookie?.split(';', 1)[0] ?? '';
      if (!/^cortico_session=[A-Za-z0-9_.-]+$/.test(cookie)) throw new Error('WebUI did not return a valid session cookie');
      this.cookie = cookie;
      response = await this.request(STATE_PATH);
    }
    if (response.status !== 200) throw new Error(`Voice state unavailable (HTTP ${response.status})`);
    try { return parseVoiceSnapshot(JSON.parse(response.text)); }
    catch { throw new Error('Unsupported Coopanion voice-state schema; muting is disabled'); }
  }

  private async request(path: string, password?: string): Promise<{ status: number; text: string; cookie: string | null }> {
    const controller = new AbortController();
    this.requests.add(controller);
    const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
    try {
      const response = await fetch(this.origin + path, {
        method: password === undefined ? 'GET' : 'POST',
        headers: {
          Accept: 'application/json', Origin: this.origin,
          ...(this.cookie ? { Cookie: this.cookie } : {}),
          ...(password === undefined ? {} : { 'Content-Type': 'application/json' }),
        },
        ...(password === undefined ? {} : { body: JSON.stringify({ password }) }),
        redirect: 'error',
        signal: controller.signal,
      });
      const chunks: Uint8Array[] = [];
      let size = 0;
      if (response.body) {
        for await (const chunk of response.body) {
          size += chunk.length;
          if (size > BODY_LIMIT) { controller.abort(); throw new Error('Voice response exceeds size limit'); }
          chunks.push(chunk);
        }
      }
      return { status: response.status, text: Buffer.concat(chunks).toString('utf8'), cookie: response.headers.get('set-cookie') };
    } catch {
      throw new Error('Local WebUI request failed or timed out');
    } finally {
      clearTimeout(timeout);
      this.requests.delete(controller);
    }
  }
}
