import { writeFileSync } from 'node:fs';
import { setTimeout as delay } from 'node:timers/promises';

const mode = process.argv[2], trace = process.argv[3];
const messages = [];
let stopped = false, outputBroken = false, heartbeat;
let sequence = 0;
process.stdout.on('error', () => { outputBroken = true; });
const send = (message) => { if (!outputBroken) process.stdout.write(JSON.stringify(message) + '\n'); };
const status = (active) => ({
  type: 'status', active, owned: active ? 1 : 0, pendingRestore: 0,
  overrides: 0, suspended: null, recoveryUncertain: false,
  captureId: active ? 'recording' : null, prepared: active,
  directHotkey: false,
});
function finish() {
  if (stopped) return;
  stopped = true;
  clearInterval(heartbeat);
  writeFileSync(trace, JSON.stringify(messages));
  send(status(false));
  if (outputBroken || process.stdout.destroyed) process.exit(0);
  process.stdout.write('{"type":"stopped","restored":true,"pendingRestore":0}\n', () => process.exit(0));
}
let input = '';
process.stdin.on('data', (chunk) => {
  input += String(chunk);
  let newline;
  while ((newline = input.indexOf('\n')) >= 0) {
    const message = JSON.parse(input.slice(0, newline));
    input = input.slice(newline + 1);
    messages.push(message);
    if (message.type === 'shutdown') finish();
  }
});
process.stdin.on('end', finish);
send({ type: 'ready', protocol: 6, backend: 'simulation' });
send(status(true));
if (mode !== 'silent') {
  heartbeat = setInterval(() => send({ type: 'heartbeat', protocol: 6, sequence: sequence++ }), 200);
}
await delay(250);
if (mode === 'flood') {
  const bytes = Buffer.alloc(64 * 1024, 0x78);
  for (let n = 0; n < 128 && !stopped && !outputBroken; n++) {
    if (!process.stdout.write(bytes)) {
      await new Promise((resolve) => {
        process.stdout.once('drain', resolve);
        process.stdout.once('error', resolve);
      });
    }
  }
} else if (mode === 'batch') {
  process.stdout.write(Array.from({ length: 150 }, () => JSON.stringify(status(false))).join('\n') + '\n');
} else if (mode === 'bad-status') {
  send({ ...status(false), owned: -1 });
} else if (mode === 'duplicate-ready') {
  send({ type: 'ready', protocol: 6, backend: 'simulation' });
} else if (mode === 'bad-sequence') {
  send({ type: 'heartbeat', protocol: 6, sequence: -1 });
} else if (mode === 'bad-latency') {
  send({ type: 'latency', value: { source: 'hook', dispatchMs: 10, firstMuteMs: 1, allMutedMs: 2 } });
} else if (mode === 'latency') {
  for (let n = 0; n < 40; n++) {
    send({ type: 'latency', value: { source: 'simulation', dispatchMs: 1, firstMuteMs: 3, allMutedMs: 10 + n } });
  }
} else if (mode === 'slow-failure') {
  for (let n = 0; n < 40; n++) send({ type: 'diagnostic', phase: 'journal.save', ms: 25 + n, ok: true });
  send({ type: 'error', message: 'Audio worker unresponsive during journal.save' });
} else if (mode === 'slow-success') {
  send({ type: 'diagnostic', phase: 'audio.read', ms: 400, ok: true });
} else if (mode === 'truncated') {
  const frame = Buffer.from(JSON.stringify({ ...status(false), suspended: '\u6b63\u5e38' }) + '\n');
  const split = frame.indexOf(Buffer.from('\u6b63')) + 1;
  process.stdout.write(frame.subarray(0, split));
  await delay(50);
  process.stdout.write(frame.subarray(split));
}
