import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, copyFileSync } from 'node:fs';
import { delimiter, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));

export function runCargo(args) {
  const sysroot = spawnSync('rustc', ['--print', 'sysroot'], { cwd: root, encoding: 'utf8', windowsHide: true });
  if (sysroot.error || sysroot.status !== 0) throw new Error('Install Rust with rustup, then install the toolchain in rust-toolchain.toml');
  const tools = join(sysroot.stdout.trim(), 'lib', 'rustlib', 'x86_64-pc-windows-gnu', 'bin');
  const selfContained = join(tools, 'self-contained');
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => key.toLowerCase() !== 'path'));
  env.Path = [
    ...(existsSync(selfContained) ? [selfContained, tools, join(sysroot.stdout.trim(), 'bin')] : []),
    process.env.PATH ?? process.env.Path ?? '',
  ].join(delimiter);
  if (existsSync(selfContained)) {
    const llvmAr = join(tools, 'llvm-ar.exe');
    if (!existsSync(llvmAr)) throw new Error('Run rustup component add llvm-tools-preview');
    const alias = join(root, 'scratch', 'tools', 'llvm-dlltool.exe');
    mkdirSync(join(root, 'scratch', 'tools'), { recursive: true });
    // LLVM's archiver dispatches to its DLL import-library driver by executable name.
    // Unlike GNU dlltool, this driver does not require an external assembler.
    copyFileSync(llvmAr, alias);
    if (env.RUSTFLAGS && !env.CARGO_ENCODED_RUSTFLAGS) {
      throw new Error('Use CARGO_ENCODED_RUSTFLAGS instead of RUSTFLAGS to preserve paths containing spaces');
    }
    env.CARGO_ENCODED_RUSTFLAGS = [
      ...(env.CARGO_ENCODED_RUSTFLAGS?.split('\x1f') ?? []),
      '-C', `dlltool=${alias}`,
      '-C', `linker=${join(tools, 'rust-lld.exe')}`,
      '-C', 'linker-flavor=ld.lld',
    ].join('\x1f');
  }
  const result = spawnSync('cargo', args, { cwd: root, env, stdio: 'inherit', windowsHide: true, shell: false });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) runCargo(process.argv.slice(2));
