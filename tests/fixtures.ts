import { mkdirSync, mkdtempSync, readFileSync, writeFileSync, renameSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { WebSocketServer } from 'ws';

export const ROOT = fileURLToPath(new URL('../', import.meta.url));

export function scratch(prefix: string): string {
  const directory = join(ROOT, 'scratch', 'tests');
  mkdirSync(directory, { recursive: true });
  return mkdtempSync(join(directory, prefix));
}

export function writeJson(path: string, data: unknown): void {
  const temporary = path + '.test-tmp';
  writeFileSync(temporary, JSON.stringify(data));
  renameSync(temporary, path);
}

export function readJson(path: string): any {
  return JSON.parse(readFileSync(path, 'utf8'));
}

export async function until(predicate: () => boolean | Promise<boolean>, timeoutMs = 5000): Promise<void> {
  const start = performance.now();
  while (!await predicate()) {
    if (performance.now() - start > timeoutMs) throw new Error('Condition timed out');
    await delay(20);
  }
}

export function state(open = false) {
  return {
    enabled: true,
    server: { phase: 'running' },
    mic: { state: 'on' },
    input: { mode: 'hold', open, openOnPress: false, hotkeyProblem: null, hotkey: 'LeftAlt*2' },
  };
}

export async function voiceServer(options: { password?: string; delayMs?: number } = {}) {
  let snapshot: unknown = state();
  let enabled = true;
  let delayMs = options.delayMs ?? 0;
  const requests: Array<{ url: string; cookie: string | undefined }> = [];
  const authorized = (req: IncomingMessage) => !options.password || req.headers.cookie === 'cortico_session=test_session';
  const server = createServer((req: IncomingMessage, res: ServerResponse) => {
    requests.push({ url: req.url ?? '', cookie: req.headers.cookie });
    if (req.url === '/api/auth/login' && req.method === 'POST') {
      let body = '';
      req.on('data', (bytes) => { body += String(bytes); });
      req.on('end', () => {
        if (JSON.parse(body).password !== options.password) { res.writeHead(401).end('{}'); return; }
        res.setHeader('Set-Cookie', 'cortico_session=test_session; HttpOnly; SameSite=Strict');
        res.end('{"ok":true}');
      });
      return;
    }
    if (!authorized(req)) { res.writeHead(401).end('{}'); return; }
    if (req.url !== '/api/webui/providers/world%3Adesktop-pet/panels/voice/state') { res.writeHead(404).end('{}'); return; }
    if (!enabled) { res.writeHead(503).end('{}'); return; }
    const text = JSON.stringify(snapshot);
    const respond = () => { if (!res.destroyed) { res.setHeader('Content-Type', 'application/json'); res.end(text); } };
    if (delayMs) setTimeout(respond, delayMs);
    else respond();
  });
  const wss = new WebSocketServer({ noServer: true });
  server.on('upgrade', (req, socket, head) => {
    if (!authorized(req) || req.url !== '/ws/providers/world%3Adesktop-pet/panels/voice') {
      socket.end('HTTP/1.1 401 Unauthorized\r\n\r\n'); return;
    }
    wss.handleUpgrade(req, socket, head, (ws) => wss.emit('connection', ws, req));
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('Missing server port');
  return {
    origin: `http://127.0.0.1:${address.port}`,
    requests, wss,
    snapshot(value: unknown) { snapshot = value; },
    availability(value: boolean) { enabled = value; },
    latency(ms: number) { delayMs = ms; },
    send(message: unknown) { for (const client of wss.clients) client.send(JSON.stringify(message)); },
    gate(open: boolean) { snapshot = state(open); this.send({ type: 'gate', open }); },
    async close() {
      for (const client of wss.clients) client.terminate();
      await new Promise<void>((resolve) => wss.close(() => resolve()));
      server.closeAllConnections();
      await new Promise<void>((resolve) => server.close(() => resolve()));
    },
  };
}
