import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, lstatSync, realpathSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = realpathSync(join(dirname(fileURLToPath(import.meta.url)), '..'));
const run = (program, args) => execFileSync(program, args, { cwd: root, encoding: 'utf8' });
const target = process.env.TAURI_ENV_TARGET_TRIPLE
  || run('rustc', ['-vV']).match(/^host: (.+)$/m)?.[1];
if (!target || !/^[a-z0-9_-]+$/.test(target)) throw new Error('Invalid Rust target triple.');
if (!target.endsWith('-apple-darwin')) {
  if (!target.includes('-windows-') && !target.endsWith('-linux-gnu')) throw new Error('Unsupported clock helper target.');
  const manifest = join(root, 'engine-rs', 'Cargo.toml');
  const debug = process.argv.slice(2).includes('--debug');
  const metadata = JSON.parse(run('cargo', ['metadata', '--manifest-path', manifest, '--no-deps', '--format-version', '1']));
  execFileSync('cargo', ['build', '--manifest-path', manifest, '--bin', 'agent-manager-clock-helper', '--target', target,
    ...(debug ? [] : ['--release'])], { cwd: root, stdio: 'inherit' });
  const filename = `agent-manager-clock-helper${target.includes('-windows-') ? '.exe' : ''}`;
  const directory = join(root, 'app', 'src-tauri', 'clock-helper');
  if (realpathSync(directory) !== directory) throw new Error('Clock helper resource directory must not be a symlink.');
  const destination = join(directory, filename);
  try {
    if (!lstatSync(destination).isFile()) throw new Error('Clock helper destination must be a regular file.');
  } catch (error) { if (error.code !== 'ENOENT') throw error; }
  copyFileSync(join(metadata.target_directory, target, debug ? 'debug' : 'release', filename), destination);
  if (process.platform !== 'win32') chmodSync(destination, 0o755);
}
