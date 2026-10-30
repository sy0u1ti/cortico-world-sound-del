import { mkdirSync, copyFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { runCargo } from './rust.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
if (process.platform !== 'win32' || process.arch !== 'x64') throw new Error('Build on Windows x64 with the toolchain selected by rust-toolchain.toml');
function assertStopped() {
  const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command',
    '@(Get-CimInstance Win32_Process -Filter "Name=\'sound-del-helper.exe\'" | Select-Object -ExpandProperty ExecutablePath) | ConvertTo-Json -Compress',
  ], { encoding: 'utf8', windowsHide: true, timeout: 10_000 });
  if (result.status !== 0) throw new Error('Cannot verify running helpers before building');
  const paths = result.stdout.trim() ? JSON.parse(result.stdout) : [];
  const executable = join(root, 'bin/win32-x64/sound-del-helper.exe').toLowerCase();
  if ([paths].flat().some((path) => typeof path === 'string' && path.toLowerCase() === executable)) {
    throw new Error('Disable this installed World before rebuilding, or build in a separate checkout');
  }
}
assertStopped();
runCargo(['build', '--release', '--locked', '--manifest-path', 'native/Cargo.toml']);
assertStopped();
const target = join(root, 'bin', 'win32-x64');
mkdirSync(target, { recursive: true });
copyFileSync(join(root, 'native', 'target', 'release', 'sound-del-helper.exe'), join(target, 'sound-del-helper.exe'));
