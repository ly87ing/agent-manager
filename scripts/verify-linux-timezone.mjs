// Changes timezone only on a disposable CI machine; never run on a personal host.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import { existsSync, lstatSync, mkdtempSync, readFileSync, readlinkSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

assert.equal(process.platform, 'linux');
assert.equal(process.env.CI, 'true');
assert.equal(process.env.AM_ISOLATED_LINUX_TIMEZONE, '1');
assert.notEqual(process.getuid(), 0);
function run(command, args, env = process.env, failure = false) {
  const result = spawnSync(command, args, { env, encoding: 'utf8', timeout: 30_000, maxBuffer: 1_048_576 });
  assert.ifError(result.error);
  if (!failure) assert.equal(result.status, 0, `${command} failed: ${result.stderr}`);
  return result;
}
const originalZone = run('/usr/bin/timedatectl', ['show', '--property=Timezone', '--value']).stdout.trim();
assert.match(originalZone, /^[A-Za-z0-9_+/-]+$/);
assert.equal(run('/usr/bin/timedatectl', ['show', '--property=LocalRTC', '--value']).stdout.trim(), 'no');
const selected = originalZone === 'Europe/London' ? 'Asia/Tokyo' : 'Europe/London';
const root = mkdtempSync(join(tmpdir(), 'am-linux-timezone-'));
const home = join(root, 'missing-home');
const data = join(root, 'missing-data');
const env = { ...process.env, HOME: home, AGENT_MANAGER_DATA_DIR: data, XDG_CURRENT_DESKTOP: '' };
const binary = resolve('engine-rs/target/debug/agent-manager-engine');
const spec = JSON.stringify({ timezone: selected });
function cli(command, flags = [], failure = false, profile = spec, caller = env) {
  const result = run(binary, [command, '--spec', profile, ...flags, '--json'], caller, failure);
  const envelope = JSON.parse(result.stdout);
  if (!failure) assert.equal(envelope.ok, true, JSON.stringify(envelope.errors));
  return envelope;
}
function bus(method, ...args) {
  const output = run('/usr/bin/busctl', ['--user', '--auto-start=no', '--json=short', 'call', 'org.freedesktop.systemd1', '/org/freedesktop/systemd1', method.startsWith('Get') ? 'org.freedesktop.DBus.Properties' : 'org.freedesktop.systemd1.Manager', method, ...args]).stdout.trim();
  return output ? JSON.parse(output) : null;
}
function managerTZ() {
  const values = bus('Get', 'ss', 'org.freedesktop.systemd1.Manager', 'Environment').data[0];
  assert.equal(values.type, 'as');
  const entries = values.data.filter(value => value.startsWith('TZ='));
  assert(entries.length <= 1);
  return entries;
}
const originalTZ = managerTZ();
const zoneRoot = realpathSync('/usr/share/zoneinfo');
const zoneFiles = [...new Set([originalZone, selected, 'Pacific/Honolulu', 'Etc/UTC']
  .map(zone => realpathSync(`/usr/share/zoneinfo/${zone}`)))].map(path => {
    assert(path.startsWith(`${zoneRoot}/`));
    const meta = lstatSync(path);
    assert(meta.isFile() && meta.uid === 0);
    return { path, dev: meta.dev, ino: meta.ino, mode: meta.mode & 0o7777,
      digest: createHash('sha256').update(readFileSync(path)).digest('hex') };
  });
// Some hosted images ship globally writable tzdata. Product trust remains strict;
// the fixture verifies refusal, then temporarily establishes trusted test input.
const zonePermissions = String.raw`
import fs from 'node:fs';
import { createHash } from 'node:crypto';
const [raw, restore] = process.argv.slice(1);
const errors = [];
for (const file of JSON.parse(raw)) {
  let fd;
  try {
    fd = fs.openSync(file.path, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
    const meta = fs.fstatSync(fd);
    if (!meta.isFile() || meta.uid !== 0 || meta.dev !== file.dev || meta.ino !== file.ino
      || createHash('sha256').update(fs.readFileSync(fd)).digest('hex') !== file.digest) {
      throw Error('Changed CI zoneinfo');
    }
    const mode = meta.mode & 0o7777;
    const trusted = file.mode & ~0o022;
    // Partial preparation leaves some files unchanged. Restoration accepts only
    // the original or our exact temporary mode and still tries the other files.
    if (restore === '1' && mode === file.mode) continue;
    if (mode !== (restore === '1' ? trusted : file.mode)) {
      throw Error('Changed CI zoneinfo mode');
    }
    fs.fchmodSync(fd, restore === '1' ? file.mode : trusted);
  } catch (error) {
    if (restore !== '1') throw error;
    errors.push(error);
  } finally {
    if (fd !== undefined) fs.closeSync(fd);
  }
}
if (errors.length) throw new AggregateError(errors, 'CI zoneinfo restoration failed');
`;
const rule = `/etc/polkit-1/rules.d/49-agent-manager-ci-${randomUUID()}.rules`;
const user = run('/usr/bin/id', ['-un']).stdout.trim();
const policy = `polkit.addRule(function(action, subject) { if (action.id === "org.freedesktop.timedate1.set-timezone" && subject.user === ${JSON.stringify(user)}) return polkit.Result.YES; });\n`;
// This is CI fixture setup, not product authorization. The file is exclusively
// created and removed only on an exact readback match.
function rootNode(source, args) {
  return run('/usr/bin/sudo', ['-n', process.execPath, '--input-type=module', '-e', source, ...args]);
}
function collected(child) {
  return new Promise((resolve, reject) => {
    let stdout = '', stderr = '';
    const timer = setTimeout(() => reject(new Error('Owned CI child exceeded its deadline')), 40_000);
    child.stdout.on('data', chunk => { stdout += chunk; if (stdout.length > 1_048_576) reject(new Error('Owned child output exceeded limit')); });
    child.stderr.on('data', chunk => { stderr += chunk; if (stderr.length > 1_048_576) reject(new Error('Owned child error exceeded limit')); });
    child.on('error', error => { clearTimeout(timer); reject(error); });
    child.on('close', status => { clearTimeout(timer); resolve({ status, stdout, stderr }); });
  });
}
let installed = false;
let installedPolicy = policy;
let touched = false;
let preparedZones = false;
async function authenticate(mode) {
  const preview = cli('environment:repair-preview').data;
  const child = spawn('/bin/sh', ['-c', 'kill -STOP $$; exec "$@"', 'am-owned-cli', binary,
    'environment:repair', '--spec', spec, '--base', preview.base, '--confirm', '--json'],
    { env, stdio: ['ignore', 'pipe', 'pipe'] });
  const childResult = collected(child);
  let agent, agentResult, ready;
  try {
    const stoppedBy = Date.now() + 5_000;
    let fields;
    do {
      fields = readFileSync(`/proc/${child.pid}/stat`, 'utf8').split(')').at(-1).trim().split(/\s+/);
      if (fields[0] === 'T') break;
      assert(Date.now() < stoppedBy, 'Owned child did not stop before authentication');
      await delay(20);
    } while (true);
    const current = installedPolicy;
    const challenge = `polkit.addRule(function(action, subject) { if (action.id === "org.freedesktop.timedate1.set-timezone" && subject.user === ${JSON.stringify(user)} && subject.pid === ${child.pid}) return polkit.Result.AUTH_SELF_KEEP; });\n`;
    rootNode("import fs from 'node:fs'; const [p,old,t]=process.argv.slice(1); if(fs.readFileSync(p,'utf8')!==old) throw Error('Foreign policy'); fs.writeFileSync(p,t);", [rule, current, challenge]);
    installedPolicy = challenge;
    const subject = `${child.pid},${fields[19]},${process.getuid()}`;
    const policyBy = Date.now() + 10_000;
    let check;
    do {
      check = run('/usr/bin/pkcheck', ['--action-id', 'org.freedesktop.timedate1.set-timezone', '--process', subject], process.env, true).status;
      if (check === 2) break;
      assert([0, 1].includes(check), 'CI challenge query failed');
      assert(Date.now() < policyBy, 'Exact-process authentication challenge was not loaded');
      await delay(100);
    } while (true);
    const example = resolve('engine-rs/target/debug/examples/linux_polkit_fixture');
    agent = spawn('/usr/bin/sudo', ['-n', '/usr/bin/env', 'CI=true', 'AM_ISOLATED_LINUX_TIMEZONE=1', example,
      String(process.getuid()), String(child.pid), fields[19], mode], { stdio: ['ignore', 'pipe', 'pipe'] });
    agentResult = collected(agent);
    let output = '';
    const readyPromise = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('CI authentication agent was not ready')), 10_000);
      agent.stdout.on('data', chunk => {
        output += chunk;
        if (output.includes('\n')) {
          clearTimeout(timer);
          try { ready = JSON.parse(output.slice(0, output.indexOf('\n'))); resolve(); } catch (error) { reject(error); }
        }
      });
      agent.on('close', () => { clearTimeout(timer); reject(new Error('CI authentication agent exited before registration')); });
    });
    await readyPromise;
    assert.equal(ready.ready, true);
    process.kill(child.pid, 'SIGCONT');
    const result = await childResult;
    assert.equal(result.status, 0, `Authenticated CLI failed: ${result.stderr}`);
    const receipt = await agentResult;
    assert.equal(receipt.status, 0, `CI authentication protocol failed: ${receipt.stderr}`);
    return JSON.parse(result.stdout).data;
  } finally {
    // Only these two explicitly owned fixture processes can be terminated.
    if (child.exitCode === null) { try { child.kill('SIGKILL'); } catch {} }
    await childResult.catch(() => {});
    if (ready && agent.exitCode === null) {
      rootNode("import fs from 'node:fs'; const [pid,start]=process.argv.slice(1); try { const stat=fs.readFileSync('/proc/'+pid+'/stat','utf8').split(')').at(-1).trim().split(/\\s+/); if(stat[19]===start) process.kill(Number(pid),'SIGTERM'); } catch(e) { if(e.code!=='ENOENT') throw e; }", [String(ready.pid), ready.start]);
    }
    if (agentResult) await agentResult.catch(() => {});
  }
}
try {
  if (zoneFiles.find(file => file.path === realpathSync(`/usr/share/zoneinfo/${originalZone}`)).mode & 0o022) {
    const unsafe = cli('environment:repair-preview').data;
    assert.equal(unsafe.items.find(item => item.id === 'time.timezone').status, 'unavailable');
    assert(!existsSync(home));
    assert(!existsSync(data));
  }
  preparedZones = true;
  rootNode(zonePermissions, [JSON.stringify(zoneFiles), '0']);
  rootNode("import fs from 'node:fs'; const [p,t]=process.argv.slice(1); const fd=fs.openSync(p,'wx',0o644); fs.writeFileSync(fd,t); fs.closeSync(fd);", [rule, policy]);
  installed = true;
  const start = readFileSync('/proc/self/stat', 'utf8').split(')').at(-1).trim().split(/\s+/)[19];
  const subject = `${process.pid},${start},${process.getuid()}`;
  const deadline = Date.now() + 10_000;
  while (run('/usr/bin/pkcheck', ['--action-id', 'org.freedesktop.timedate1.set-timezone', '--process', subject], process.env, true).status !== 0) {
    assert(Date.now() < deadline, 'Disposable authorization rule was not loaded');
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
  }
  const preview = cli('environment:repair-preview').data;
  const item = preview.items.find(item => item.id === 'time.timezone');
  if (item.before !== originalZone) {
  const raw = JSON.parse(run('/usr/bin/busctl', ['--system', '--json=short', 'call', 'org.freedesktop.timedate1', '/org/freedesktop/timedate1', 'org.freedesktop.DBus.Properties', 'GetAll', 's', 'org.freedesktop.timedate1']).stdout);
  const meta = lstatSync('/etc/localtime');
  const targetMeta = lstatSync(realpathSync('/etc/localtime'));
  console.log(JSON.stringify({ timezone: raw.data?.[0]?.Timezone, rtc: raw.data?.[0]?.LocalRTC,
    signature: raw.type, count: raw.data?.length, target_owner: targetMeta.uid, target_mode: targetMeta.mode.toString(8),
    symlink: meta.isSymbolicLink(), owner: meta.uid,
    link: meta.isSymbolicLink() ? readlinkSync('/etc/localtime') : null,
    adjtime: existsSync('/etc/adjtime') ? run('/usr/bin/sudo', ['-n', '/usr/bin/cat', '/etc/adjtime']).stdout : null }));
  }
  assert.equal(item.before, originalZone);
  assert.equal(item.status, 'change');
  assert(!existsSync(home));
  assert(!existsSync(data));
  assert.equal(run('/usr/bin/timedatectl', ['show', '--property=Timezone', '--value']).stdout.trim(), originalZone);
  touched = true;
  const applied = cli('environment:repair', ['--base', preview.base, '--confirm']).data;
  assert.equal(applied.status, 'applied');
  assert.equal(applied.items.find(item => item.id === 'time.timezone').status, 'verified');
  assert.equal(realpathSync('/etc/localtime'), realpathSync(`/usr/share/zoneinfo/${selected}`));
  const db = join(data, 'native-manager.db');
  const before = readFileSync(db);
  const fresh = cli('environment:repair-preview').data;
  assert.equal(cli('environment:repair', ['--base', fresh.base, '--confirm']).data.status, 'not_needed');
  assert.deepEqual(readFileSync(db), before);

  run('/usr/bin/sudo', ['-n', '/usr/bin/timedatectl', 'set-timezone', originalZone]);
  const stale = cli('environment:repair', ['--base', fresh.base, '--confirm'], true);
  assert.equal(stale.ok, false);
  assert(stale.errors.some(error => error.message.includes('changed after preview')));
  assert.deepEqual(readFileSync(db), before);

  // Deliberately diverge the disk from the still-running timedated cache.
  run('/usr/bin/sudo', ['-n', '/usr/bin/ln', '-sfn', `/usr/share/zoneinfo/${selected}`, '/etc/localtime']);
  const mismatch = cli('environment:repair-preview').data;
  assert.equal(mismatch.items.find(item => item.id === 'time.timezone').status, 'unavailable');
  assert.deepEqual(readFileSync(db), before);
  assert(!existsSync(home));
  run('/usr/bin/sudo', ['-n', '/usr/bin/timedatectl', 'set-timezone', selected]);
  run('/usr/bin/sudo', ['-n', '/usr/bin/timedatectl', 'set-timezone', originalZone]);
  bus('SetEnvironment', 'as', '1', 'TZ=Etc/UTC');
  const deniedPolicy = policy.replace('polkit.Result.YES', 'polkit.Result.NO');
  rootNode("import fs from 'node:fs'; const [p,old,t]=process.argv.slice(1); if(fs.readFileSync(p,'utf8')!==old) throw Error('Foreign policy'); fs.writeFileSync(p,t);", [rule, installedPolicy, deniedPolicy]);
  installedPolicy = deniedPolicy;
  const denyDeadline = Date.now() + 10_000;
  let denyCode;
  while ((denyCode = run('/usr/bin/pkcheck', ['--action-id', 'org.freedesktop.timedate1.set-timezone', '--process', subject], process.env, true).status) === 0) {
    assert(Date.now() < denyDeadline, 'Disposable deny policy was not loaded');
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
  }
  assert.equal(denyCode, 1, 'Policy denial must be explicit, not an authorization error');
  const deniedPreview = cli('environment:repair-preview').data;
  const denied = cli('environment:repair', ['--base', deniedPreview.base, '--confirm']).data;
  assert.equal(denied.status, 'partial');
  assert.equal(denied.items.find(item => item.id === 'time.timezone').status, 'failed');
  assert.equal(realpathSync('/etc/localtime'), realpathSync(`/usr/share/zoneinfo/${originalZone}`));
  assert.deepEqual(managerTZ(), ['TZ=Etc/UTC'], 'Denied authorization continued to session writes');
  const cached = await authenticate('allow');
  assert.equal(cached.status, 'applied');
  assert.equal(cached.items.find(item => item.id === 'time.timezone').status, 'verified');
  assert.equal(realpathSync('/etc/localtime'), realpathSync(`/usr/share/zoneinfo/${selected}`));
  run('/usr/bin/sudo', ['-n', '/usr/bin/timedatectl', 'set-timezone', originalZone]);
  bus('SetEnvironment', 'as', '1', 'TZ=Etc/UTC');
  const changed = await authenticate('change');
  assert.equal(changed.status, 'partial');
  assert.equal(changed.items.find(item => item.id === 'time.timezone').status, 'stale');
  assert.equal(realpathSync('/etc/localtime'), realpathSync('/usr/share/zoneinfo/Pacific/Honolulu'));
  assert.deepEqual(managerTZ(), ['TZ=Etc/UTC'], 'Changed authorization baseline continued to session writes');
  console.log('Linux timezone: actual Polkit challenge/cache, apply, disk readback, no-op, stale, cache disagreement and denied authorization passed.');

  // Real GSettings uses an isolated keyfile backend. The owned D-Bus name proves
  // process identity/policy gates, not GNOME/GeoClue desktop behavior.
  run('/usr/bin/sudo', ['-n', '/usr/bin/timedatectl', 'set-timezone', originalZone]);
  const desktopEnv = { ...env, XDG_CURRENT_DESKTOP: 'GNOME', GSETTINGS_BACKEND: 'keyfile',
    XDG_CONFIG_HOME: join(root, 'gnome-config') };
  function gsettings(verb, schema, key, value) {
    return run('/usr/bin/gsettings', [verb, schema, key, ...(value === undefined ? [] : [value])], desktopEnv).stdout.trim();
  }
  function automatic(value) {
    return gsettings('set', 'org.gnome.desktop.datetime', 'automatic-timezone', String(value));
  }
  function location(value) {
    return gsettings('set', 'org.gnome.system.location', 'enabled', String(value));
  }
  function readAutomatic() {
    return gsettings('get', 'org.gnome.desktop.datetime', 'automatic-timezone');
  }
  function replacePolicy(next) {
    rootNode("import fs from 'node:fs'; const [p,old,t]=process.argv.slice(1); if(fs.readFileSync(p,'utf8')!==old) throw Error('Foreign policy'); fs.writeFileSync(p,t);", [rule, installedPolicy, next]);
    installedPolicy = next;
  }
  async function loadedPolicy(allowed) {
    const by = Date.now() + 10_000;
    while (run('/usr/bin/pkcheck', ['--action-id', 'org.freedesktop.timedate1.set-timezone', '--process', subject], process.env, true).status !== (allowed ? 0 : 1)) {
      assert(Date.now() < by, 'Disposable desktop policy was not loaded');
      await delay(100);
    }
  }
  const desktop = spawn(resolve('engine-rs/target/debug/examples/linux_gnome_fixture'), [],
    { env: { ...desktopEnv, AM_ISOLATED_LINUX_TIMEZONE: '1' }, stdio: ['ignore', 'pipe', 'pipe'] });
  const desktopResult = collected(desktop);
  // Keep timeout rejection observed while the other fixture assertions run;
  // the original result is still awaited and only this owned child is cleaned.
  void desktopResult.catch(() => {});
  try {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('Owned desktop fixture did not register')), 10_000);
      desktop.stdout.once('data', chunk => {
        clearTimeout(timer);
        try { assert.equal(JSON.parse(String(chunk)).ready, true); resolve(); } catch (error) { reject(error); }
      });
      desktop.once('close', () => { clearTimeout(timer); reject(new Error('Owned desktop fixture exited')); });
    });
    automatic(true);
    location(false);
    const combined = JSON.stringify({ timezone: selected, automatic_timezone: false });
    replacePolicy(deniedPolicy);
    await loadedPolicy(false);
    const rejected = cli('environment:repair-preview', [], false, combined, desktopEnv).data;
    assert.equal(rejected.items.find(item => item.id === 'time.automatic_timezone').scope, 'user');
    const deniedBatch = cli('environment:repair', ['--base', rejected.base, '--confirm'], false, combined, desktopEnv).data;
    assert.equal(deniedBatch.status, 'partial');
    assert.equal(readAutomatic(), 'true', 'Authorization denial wrote a user preference');
    assert.equal(realpathSync('/etc/localtime'), realpathSync(`/usr/share/zoneinfo/${originalZone}`));
    assert.deepEqual(managerTZ(), ['TZ=Etc/UTC']);

    const disable = JSON.stringify({ automatic_timezone: false });
    const disablePreview = cli('environment:repair-preview', [], false, disable, desktopEnv).data;
    assert.equal(disablePreview.items[0].status, 'change', 'Privacy or daemon denial blocked explicit disable');
    const disabled = cli('environment:repair', ['--base', disablePreview.base, '--confirm'], false, disable, desktopEnv).data;
    assert.equal(disabled.status, 'applied');
    assert.equal(disabled.restart_required, false);
    assert.equal(readAutomatic(), 'false');
    automatic(true);

    replacePolicy(policy);
    await loadedPolicy(true);
    const ordered = cli('environment:repair-preview', [], false, combined, desktopEnv).data;
    assert.deepEqual(ordered.items.slice(0, 2).map(item => item.id), ['time.automatic_timezone', 'time.timezone']);
    const fixed = cli('environment:repair', ['--base', ordered.base, '--confirm'], false, combined, desktopEnv).data;
    assert.equal(fixed.status, 'applied');
    assert(fixed.items.every(item => ['verified', 'unchanged'].includes(item.status)));
    assert.equal(readAutomatic(), 'false');
    assert.equal(realpathSync('/etc/localtime'), realpathSync(`/usr/share/zoneinfo/${selected}`));
    const saved = readFileSync(db);
    const noop = cli('environment:repair-preview', [], false, combined, desktopEnv).data;
    assert.equal(cli('environment:repair', ['--base', noop.base, '--confirm'], false, combined, desktopEnv).data.status, 'not_needed');
    assert.deepEqual(readFileSync(db), saved);
    automatic(true);
    assert.equal(cli('environment:repair', ['--base', noop.base, '--confirm'], true, combined, desktopEnv).ok, false);
    assert.equal(readAutomatic(), 'true');
    assert.deepEqual(readFileSync(db), saved);

    const enable = JSON.stringify({ automatic_timezone: true });
    automatic(false);
    let unavailable = cli('environment:repair-preview', [], false, enable, desktopEnv).data;
    assert.equal(unavailable.items[0].status, 'unavailable');
    assert.equal(cli('environment:repair', ['--base', unavailable.base, '--confirm'], false, enable, desktopEnv).data.status, 'partial');
    assert.deepEqual(readFileSync(db), saved);
    assert.equal(gsettings('get', 'org.gnome.system.location', 'enabled'), 'false');
    location(true);
    replacePolicy(deniedPolicy);
    await loadedPolicy(false);
    unavailable = cli('environment:repair-preview', [], false, enable, desktopEnv).data;
    assert.equal(unavailable.items[0].status, 'unavailable', 'Denied daemon policy allowed automatic enable');
    replacePolicy(policy);
    await loadedPolicy(true);
    const enabled = cli('environment:repair-preview', [], false, enable, desktopEnv).data;
    assert.equal(enabled.items[0].status, 'change');
    const enabledResult = cli('environment:repair', ['--base', enabled.base, '--confirm'], false, enable, desktopEnv).data;
    assert.equal(enabledResult.status, 'applied');
    assert.equal(enabledResult.restart_required, false, 'Automatic timezone alone does not require restarting apps');
    assert(enabledResult.verification_limits.some(value => value.includes('geolocation')));
    assert.equal(readAutomatic(), 'true');
    const competing = cli('environment:repair-preview', [], false,
      JSON.stringify({ timezone: originalZone, automatic_timezone: true }), desktopEnv).data;
    assert.equal(competing.items.find(item => item.id === 'time.timezone').status, 'unavailable', 'Fixed-zone repair competed with enabled automatic timezone');

    const beforeExit = cli('environment:repair-preview', [], false, JSON.stringify({ automatic_timezone: false }), desktopEnv).data;
    desktop.kill('SIGTERM');
    await desktopResult;
    assert.equal(cli('environment:repair', ['--base', beforeExit.base, '--confirm'], true,
      JSON.stringify({ automatic_timezone: false }), desktopEnv).ok, false);
    assert.equal(readAutomatic(), 'true');
    assert(!existsSync(home));
    console.log('GNOME preference fixture: caller-scoped ordering, denial zero-write, real GSettings readback, no-op, stale, privacy and daemon-policy gates passed; geolocation is not exercised.');
  } finally {
    if (desktop.exitCode === null) desktop.kill('SIGTERM');
    await desktopResult.catch(() => {});
  }
} finally {
  const cleanupErrors = [];
  try {
  if (touched) {
    // Force a transition first: SetTimezone may otherwise return early on its cache.
    run('/usr/bin/sudo', ['-n', '/usr/bin/timedatectl', 'set-timezone', selected]);
    run('/usr/bin/sudo', ['-n', '/usr/bin/timedatectl', 'set-timezone', originalZone]);
    assert.equal(realpathSync('/etc/localtime'), realpathSync(`/usr/share/zoneinfo/${originalZone}`));
    bus('UnsetAndSetEnvironment', 'asas', '1', 'TZ', String(originalTZ.length), ...originalTZ);
    assert.deepEqual(managerTZ(), originalTZ);
  }
  } catch (error) { cleanupErrors.push(error); }
  try {
    if (preparedZones) rootNode(zonePermissions, [JSON.stringify(zoneFiles), '1']);
  } catch (error) { cleanupErrors.push(error); }
  try {
    if (installed) rootNode("import fs from 'node:fs'; const [p,t]=process.argv.slice(1); if(fs.readFileSync(p,'utf8')!==t) throw Error('Foreign policy'); fs.unlinkSync(p);", [rule, installedPolicy]);
  } catch (error) { cleanupErrors.push(error); }
  if (cleanupErrors.length) throw new AggregateError(cleanupErrors, 'Disposable fixture restoration failed');
  rmSync(root, { recursive: true });
}
