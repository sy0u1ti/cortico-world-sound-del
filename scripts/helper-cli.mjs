import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const mode = process.argv[2];
if (!['--probe', '--recover-only'].includes(mode)) throw new Error('Expected --probe or --recover-only');
const exe = fileURLToPath(new URL('../bin/win32-x64/sound-del-helper.exe', import.meta.url));
const args = [mode];
if (mode !== '--probe') {
  if (!process.env.LOCALAPPDATA) throw new Error('LOCALAPPDATA is not set');
  args.push('--data-dir', join(process.env.LOCALAPPDATA, 'CoopanionSoundDel'));
}
const child = spawn(exe, args, { stdio: ['ignore', 'inherit', 'inherit'], windowsHide: true, shell: false });
child.on('error', () => { console.error('Cannot start the helper. Run npm run build first.'); process.exitCode = 1; });
child.on('exit', (code) => { process.exitCode = code ?? 1; });
