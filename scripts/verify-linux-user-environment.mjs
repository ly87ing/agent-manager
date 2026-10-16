// Writes only regional variables on a disposable CI user's existing systemd manager.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

assert.equal(process.platform, 'linux');
assert.equal(process.env.CI, 'true');
assert.equal(process.env.AM_ISOLATED_LINUX_USER_MANAGER, '1');
assert.notEqual(process.getuid(), 0);
const keys = ['LANG', 'LC_ALL', 'LC_CTYPE', 'LC_COLLATE', 'LC_MESSAGES', 'LC_MONETARY', 'LC_NUMERIC', 'LC_TIME'];
function run(command, args, env = process.env, allowedFailure = false) {
  const result = spawnSync(command, args, { env, encoding: 'utf8', timeout: 30_000, maxBuffer: 1_048_576 });
  assert.ifError(result.error);
  if (!allowedFailure) assert.equal(result.status, 0, `${command} failed: ${result.stderr}`);
  return result;
}
function bus(destination, path, iface, method, ...args) {
  const output = run('/usr/bin/busctl', ['--user', '--auto-start=no', '--json=short', 'call', destination, path, iface, method, ...args]).stdout.trim();
  return output ? JSON.parse(output) : null;
}
function identity() {
  const owner = bus('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'GetNameOwner', 's', 'org.freedesktop.systemd1');
  const id = bus('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'GetId');
  assert.equal(owner.type, 's');
  assert.equal(id.type, 's');
  assert.equal(owner.data.length, 1);
  assert.equal(id.data.length, 1);
  return [id.data[0], owner.data[0]];
}
const originalIdentity = identity();
const manager = originalIdentity[1];
function environment() {
  assert.deepEqual(identity(), originalIdentity, 'Disposable user manager changed');
  const value = bus(manager, '/org/freedesktop/systemd1', 'org.freedesktop.DBus.Properties', 'Get', 'ss', 'org.freedesktop.systemd1.Manager', 'Environment');
  assert.equal(value.type, 'v');
  assert.equal(value.data[0].type, 'as');
  const selected = new Map();
  for (const entry of value.data[0].data) {
    const split = entry.indexOf('=');
    const key = entry.slice(0, split);
    if (keys.includes(key)) {
      assert(!selected.has(key));
      selected.set(key, entry.slice(split + 1));
    }
  }
  assert.deepEqual(identity(), originalIdentity);
  return selected;
}
function set(entries) {
  assert.deepEqual(identity(), originalIdentity);
  bus(manager, '/org/freedesktop/systemd1', 'org.freedesktop.systemd1.Manager', 'SetEnvironment', 'as', String(entries.length), ...entries);
}
const original = environment();
const root = mkdtempSync(join(tmpdir(), 'am-linux-user-environment-'));
const home = join(root, 'missing-home');
const data = join(root, 'missing-data');
const env = { ...process.env, HOME: home, AGENT_MANAGER_DATA_DIR: data, LANG: 'C', LC_ALL: 'C' };
const binary = resolve('engine-rs/target/debug/agent-manager-engine');
const spec = JSON.stringify({ locale: 'C.UTF-8' });
function cli(command, flags = [], allowedFailure = false) {
  const result = run(binary, [command, '--spec', spec, ...flags, '--json'], env, allowedFailure);
  const envelope = JSON.parse(result.stdout);
  if (!allowedFailure) assert.equal(envelope.ok, true, JSON.stringify(envelope.errors));
  return envelope;
}
try {
  set(keys.map(key => `${key}=C`));
  const preview = cli('environment:repair-preview').data;
  assert.equal(preview.platform, 'linux');
  assert.equal(preview.items.length, keys.length);
  assert(preview.items.every(item => item.scope === 'session' && item.status === 'change'));
  assert(!existsSync(home));
  assert(!existsSync(data));
  assert.deepEqual(environment(), new Map(keys.map(key => [key, 'C'])));

  const applied = cli('environment:repair', ['--base', preview.base, '--confirm']).data;
  assert.equal(applied.status, 'applied');
  assert(applied.items.every(item => item.status === 'verified'));
  assert.equal(applied.restart_required, true);
  assert.deepEqual(environment(), new Map(keys.map(key => [key, 'C.UTF-8'])));
  const future = run('/usr/bin/systemd-run', ['--user', '--wait', '--pipe', '--collect', '--quiet', '/usr/bin/printenv', 'LC_ALL'], env);
  assert.equal(future.stdout.trim(), 'C.UTF-8', 'Future user unit did not inherit the manager environment');
  const variables = applied.report.checks.find(item => item.id === 'process.variables');
  assert(variables, 'Repair omitted caller evidence');
  assert.equal(JSON.parse(variables.observed).LC_ALL, 'C', 'Checker normalized itself');
  assert.equal(env.LC_ALL, 'C');

  const db = join(data, 'native-manager.db');
  const before = readFileSync(db);
  const noopPreview = cli('environment:repair-preview').data;
  const noop = cli('environment:repair', ['--base', noopPreview.base, '--confirm']).data;
  assert.equal(noop.status, 'not_needed');
  assert.deepEqual(readFileSync(db), before, 'No-op changed the database');
  set(['LC_TIME=C']);
  const stale = cli('environment:repair', ['--base', noopPreview.base, '--confirm'], true);
  assert.equal(stale.ok, false);
  assert(stale.errors.some(error => error.message.includes('changed after preview')));
  assert.equal(environment().get('LC_TIME'), 'C');
  assert.deepEqual(readFileSync(db), before, 'Stale refusal changed the database');
  assert(!existsSync(home), 'Repair wrote caller HOME');
  console.log('Linux user-manager environment: preview, apply, inheritance, no-op and stale refusal passed.');
} finally {
  assert.deepEqual(identity(), originalIdentity, 'Refusing to restore into a replaced manager');
  const entries = [...original].map(([key, value]) => `${key}=${value}`);
  bus(manager, '/org/freedesktop/systemd1', 'org.freedesktop.systemd1.Manager', 'UnsetAndSetEnvironment', 'asas', String(keys.length), ...keys, String(entries.length), ...entries);
  assert.deepEqual(environment(), original);
  rmSync(root, { recursive: true });
}
