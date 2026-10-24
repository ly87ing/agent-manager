import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, lstatSync, realpathSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = realpathSync(join(dirname(fileURLToPath(import.meta.url)), '..'));
const run = (program, args) => execFileSync(program, args, { cwd: root, encoding: 'utf8' });
const target = process.env.TAURI_ENV_TARGET_TRIPLE
  || run('rustc', ['-vV']).match(/^host: (.+)$/m)?.[1];
if (!target || !/^[a-z0-9_-]+$/.test(target)
  || !/-apple-darwin$|-windows-|-linux-gnu$/.test(target)) throw new Error('Unsupported inspection helper target.');
const manifest = join(root, 'engine-rs', 'Cargo.toml');
const debug = process.argv.slice(2).includes('--debug');
const metadata = JSON.parse(run('cargo', ['metadata', '--manifest-path', manifest, '--no-deps', '--format-version', '1']));
execFileSync('cargo', ['build', '--manifest-path', manifest, '--bin', 'agent-manager-claude-data-helper',
  '--target', target, '--locked', ...(debug ? [] : ['--release'])], { cwd: root, stdio: 'inherit' });
const filename = `agent-manager-claude-data-helper${target.includes('-windows-') ? '.exe' : ''}`;
const directory = join(root, 'app', 'src-tauri', 'claude-data-helper');
if (realpathSync(directory) !== directory) throw new Error('Inspection helper resource directory must not be a symlink.');
const destination = join(directory, filename);
try {
  const metadata = lstatSync(destination);
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.nlink !== 1) throw new Error('Inspection helper destination must be an ordinary single-link file.');
} catch (error) { if (error.code !== 'ENOENT') throw error; }
copyFileSync(join(metadata.target_directory, target, debug ? 'debug' : 'release', filename), destination);
if (process.platform !== 'win32') chmodSync(destination, 0o755);
if (target.endsWith('-apple-darwin')) {
  execFileSync('codesign', ['--force', '--sign', process.env.APPLE_SIGNING_IDENTITY || '-', '--options', 'runtime', destination], { stdio: 'inherit' });
  execFileSync('codesign', ['--verify', '--strict', destination], { stdio: 'inherit' });
}
