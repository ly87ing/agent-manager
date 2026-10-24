// Exercise an actual producer/packaged helper against owned Rust fixtures.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { lstatSync, readFileSync, realpathSync } from 'node:fs';
import { dirname, isAbsolute, join } from 'node:path';
import { homedir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { verifyExecutable, verifyHost } from './verify-native-architecture.mjs';

const [helper, expected] = process.argv.slice(2);
assert.ok(helper && process.argv.length <= 4 && isAbsolute(helper), 'Expected an absolute inspection helper path and optional target');
assert.ok(lstatSync(helper).isFile() && !lstatSync(helper).isSymbolicLink(), 'Inspection helper must be an ordinary file');
const root = realpathSync(join(dirname(fileURLToPath(import.meta.url)), '..'));
const rustHost = execFileSync('rustc', ['-vV'], { encoding: 'utf8' }).match(/^host: (\S+)$/m)?.[1];
const target = verifyHost(process.platform, process.arch,
  process.env.RUNNER_OS || { darwin: 'macOS', linux: 'Linux', win32: 'Windows' }[process.platform],
  process.env.RUNNER_ARCH || { x64: 'X64', arm64: 'ARM64' }[process.arch], rustHost, expected);
const bytes = readFileSync(helper);
verifyExecutable(bytes, target);
assert.ok(!bytes.includes(Buffer.from('sqlite_filestat')), 'Production inspector unexpectedly includes SQLite diagnostics');
for (const path of [root, homedir()]) assert.ok(!bytes.includes(Buffer.from(path)), 'Inspection helper contains an unremapped build path');
assert.ok(!process.env.AGENT_MANAGER_SQLITE_PROBE_MODE || process.env.AGENT_MANAGER_SQLITE_PROBE_MODE === 'baseline', 'Expected a normal test build');
assert.ok(!/SQLITE_(?:ENABLE_FILESTAT|DEBUG)/.test(process.env.LIBSQLITE3_FLAGS || ''), 'Expected normal SQLite compilation flags');
if (process.platform === 'darwin') execFileSync('codesign', ['--verify', '--strict', helper], { stdio: 'inherit' });
execFileSync('cargo', ['test', '--manifest-path', join(root, 'engine-rs/Cargo.toml'), '--release', '--target', target,
  '--test', 'sqlite_maintenance_probe', '--locked', '--offline'], {
  cwd: root, stdio: 'inherit', timeout: 600_000,
  env: { ...process.env, AM_TEST_INSPECTION_HELPER: helper, AGENT_MANAGER_SQLITE_PROBE_MODE: 'baseline' },
});
console.log(`Native inspection helper positive/isolation fixtures passed: ${target}`);
