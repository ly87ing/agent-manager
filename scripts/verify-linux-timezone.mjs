// Changes timezone only on a disposable CI machine; never run on a personal host.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
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
function cli(command, flags = [], failure = false) {
  const result = run(binary, [command, '--spec', spec, ...flags, '--json'], env, failure);
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
    if (installed) rootNode("import fs from 'node:fs'; const [p,t]=process.argv.slice(1); if(fs.readFileSync(p,'utf8')!==t) throw Error('Foreign policy'); fs.unlinkSync(p);", [rule, installedPolicy]);
  } catch (error) { cleanupErrors.push(error); }
  if (cleanupErrors.length) throw new AggregateError(cleanupErrors, 'Disposable fixture restoration failed');
  rmSync(root, { recursive: true });
}
