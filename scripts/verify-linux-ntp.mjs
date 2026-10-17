// Native vendor-service acceptance on a disposable GitHub runner, never a personal host.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, lstatSync, mkdtempSync, readFileSync, readlinkSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { lostReply } from './verify-linux-ntp-lost-reply.mjs';

assert.equal(process.platform, 'linux');
assert.equal(process.env.CI, 'true');
assert.equal(process.env.GITHUB_ACTIONS, 'true');
assert.equal(process.env.RUNNER_OS, 'Linux');
assert.equal(process.env.AM_ISOLATED_LINUX_NTP, '1');
assert.notEqual(process.getuid(), 0);
assert(lstatSync('/usr/bin/pkexec').isFile(), 'Native NTP acceptance requires the pkexec package');
const root = mkdtempSync(join(tmpdir(), 'am-linux-ntp-'));
const home = join(root, 'missing-home');
const data = join(root, 'data');
// The earlier workflow step starts this disposable manager; shell exports do
// not cross step boundaries. Bind the same current-UID bus for mixed-batch cases.
const runtimeDirectory = `/run/user/${process.getuid()}`;
const env = { ...process.env, HOME: home, AGENT_MANAGER_DATA_DIR: data, XDG_CURRENT_DESKTOP: '',
  XDG_RUNTIME_DIR: runtimeDirectory, DBUS_SESSION_BUS_ADDRESS: `unix:path=${runtimeDirectory}/bus` };
const binary = resolve('engine-rs/target/release/agent-manager-engine');
const agentBinary = resolve('engine-rs/target/debug/examples/linux_polkit_fixture');
const directory = '/etc/systemd/timesyncd.conf.d';
const targetFile = `${directory}/90-agent-manager.conf`;
const fixtureFile = `${directory}/80-agent-manager-ci.conf`;
const fixtureText = '# Agent Manager isolated NTP fixture v1\n[Time]\nFallbackNTP=198.51.100.2\nPollIntervalMinSec=32\nPollIntervalMaxSec=2048\n';
const selected = '127.0.0.1';
const generated = server => `# Agent Manager manual NTP configuration v1\n[Time]\nNTP=\nNTP=${server}\n`;
const spec = JSON.stringify({ ntp_server: selected });
const rule = `/etc/polkit-1/rules.d/49-agent-manager-ntp-ci-${randomUUID()}.rules`;
const user = spawnSync('/usr/bin/id', ['-un'], { encoding: 'utf8' }).stdout.trim();

function run(program, args, failure = false, caller = process.env) {
  const output = spawnSync(program, args, { env: caller, encoding: 'utf8', timeout: 150_000, maxBuffer: 1_048_576 });
  assert.ifError(output.error);
  if (!failure) assert.equal(output.status, 0, `${program}: ${output.stderr}`);
  return output;
}
function rootNode(source, args = []) {
  const syntax = spawnSync(process.execPath, ['--input-type=module', '--check'], { input: source, encoding: 'utf8', timeout: 10_000, maxBuffer: 16_384 });
  assert.ifError(syntax.error);
  assert.equal(syntax.status, 0, syntax.stderr);
  return run('/usr/bin/sudo', ['-n', process.execPath, '--input-type=module', '-e', source, ...args]);
}
function rootBus(method, signature, ...args) {
  return run('/usr/bin/sudo', ['-n', '/usr/bin/busctl', '--system', '--auto-start=no', '--json=short', 'call',
    'org.freedesktop.timesync1', '/org/freedesktop/timesync1', 'org.freedesktop.timesync1.Manager', method, signature, ...args]);
}
function unitState() {
  return run('/usr/bin/systemctl', ['is-enabled', 'systemd-timesyncd.service'], true).stdout.trim();
}
function native() {
  const raw = run('/usr/bin/busctl', ['--system', '--auto-start=no', '--json=short', 'call', 'org.freedesktop.timesync1',
    '/org/freedesktop/timesync1', 'org.freedesktop.DBus.Properties', 'GetAll', 's', 'org.freedesktop.timesync1.Manager']);
  const properties = JSON.parse(raw.stdout).data[0];
  const owner = JSON.parse(run('/usr/bin/busctl', ['--system', '--json=short', 'call', 'org.freedesktop.DBus',
    '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'GetNameOwner', 's', 'org.freedesktop.timesync1']).stdout).data[0];
  const pid = Number(run('/usr/bin/systemctl', ['show', '--property=MainPID', '--value', 'systemd-timesyncd.service']).stdout.trim());
  assert(pid > 1);
  const stat = readFileSync(`/proc/${pid}/stat`, 'utf8').split(')').at(-1).trim().split(/\s+/);
  const fields = Object.fromEntries(['RuntimeNTPServers', 'SystemNTPServers', 'LinkNTPServers', 'FallbackNTPServers',
    'RootDistanceMaxUSec', 'PollIntervalMinUSec', 'PollIntervalMaxUSec'].map(key => {
    assert(properties[key], `Missing native ${key}`);
    return [key, properties[key].data];
  }));
  return { owner, pid, birth: stat[19], enabled: unitState(), ...fields };
}
function cli(command, flags = [], failure = false, profile = spec) {
  const output = run(binary, [command, '--spec', profile, ...flags, '--json'], failure, env);
  const envelope = JSON.parse(output.stdout);
  if (!failure) assert.equal(envelope.ok, true, JSON.stringify(envelope.errors));
  return envelope;
}
function collect(child) {
  return new Promise((resolve, reject) => {
    let stdout = '', stderr = '';
    const timeout = setTimeout(() => reject(Error('Owned NTP fixture child timed out')), 150_000);
    child.stdout.on('data', chunk => { stdout += chunk; if (stdout.length > 1_048_576) reject(Error('Excess fixture output')); });
    child.stderr.on('data', chunk => { stderr += chunk; if (stderr.length > 1_048_576) reject(Error('Excess fixture error')); });
    child.on('error', error => { clearTimeout(timeout); reject(error); });
    child.on('close', status => { clearTimeout(timeout); resolve({ status, stdout, stderr }); });
  });
}
let policy;
function setPolicy(text) {
  rootNode("import fs from 'node:fs'; const [p,old,text]=process.argv.slice(1); if(old==='absent'){fs.writeFileSync(p,text,{flag:'wx',mode:0o644});}else{if(fs.readFileSync(p,'utf8')!==old)throw Error('Changed owned policy');fs.writeFileSync(p,text);}", [rule, policy ?? 'absent', text]);
  policy = text;
}
async function repair(mode, profile = spec) {
  const preview = cli('environment:repair-preview', [], false, profile).data;
  const child = spawn('/bin/sh', ['-c', 'kill -STOP $$; exec "$@"', 'am-owned-ntp-cli', binary, 'environment:repair',
    '--spec', profile, '--base', preview.base, '--confirm', '--json'], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  const output = collect(child);
  let agent, agentResult, ready;
  try {
    let fields;
    const stoppedBy = Date.now() + 5000;
    do {
      fields = readFileSync(`/proc/${child.pid}/stat`, 'utf8').split(')').at(-1).trim().split(/\s+/);
      if (fields[0] === 'T') break;
      assert(Date.now() < stoppedBy);
      await delay(20);
    } while (true);
    const probe = randomUUID();
    setPolicy(String.raw`polkit.addRule(function(action, subject) { if(action.id === "org.freedesktop.policykit.exec" && subject.user === ${JSON.stringify(user)} && subject.pid === ${child.pid} && /^\/tmp\/agent-manager-clock-[a-f0-9-]+\/clock-helper$/.test(action.lookup("program"))) return action.lookup("am-ci-probe") === ${JSON.stringify(probe)} ? polkit.Result.YES : polkit.Result.AUTH_SELF; });` + '\n');
    // Check that this exact-process rule has reached the authority before exec.
    const subject = `${child.pid},${fields[19]},${process.getuid()}`;
    const readyBy = Date.now() + 10_000;
    // Details are accepted only from trusted callers. This root-only probe has
    // an exclusive marker, so default AUTH_ADMIN cannot masquerade as readiness;
    // the actual ordinary-user pkexec still receives AUTH_SELF and a real agent.
    while (run('/usr/bin/sudo', ['-n', '/usr/bin/pkcheck', '--action-id', 'org.freedesktop.policykit.exec', '--process', subject,
      '--detail', 'program', '/tmp/agent-manager-clock-00000000-0000-0000-0000-000000000000/clock-helper', '--detail', 'am-ci-probe', probe], true).status !== 0) {
      assert(Date.now() < readyBy, 'CI NTP authorization policy was not loaded');
      await delay(100);
    }
    agent = spawn('/usr/bin/sudo', ['-n', '/usr/bin/env', 'CI=true', 'AM_ISOLATED_LINUX_NTP=1', agentBinary,
      String(process.getuid()), String(child.pid), fields[19], mode], { stdio: ['ignore', 'pipe', 'pipe'] });
    agentResult = collect(agent);
    await new Promise((resolve, reject) => {
      let text = '';
      const timeout = setTimeout(() => reject(Error('NTP authentication agent was not ready')), 10_000);
      agent.stdout.on('data', chunk => {
        text += chunk;
        if (text.includes('\n')) {
          clearTimeout(timeout);
          try { ready = JSON.parse(text.slice(0, text.indexOf('\n'))); resolve(); } catch (error) { reject(error); }
        }
      });
      agent.on('close', () => { clearTimeout(timeout); reject(Error('NTP authentication agent exited early')); });
    });
    assert.equal(ready.ready, true);
    process.kill(child.pid, 'SIGCONT');
    const observed = await output;
    assert.equal(observed.status, 0, observed.stderr);
    const envelope = JSON.parse(observed.stdout);
    assert.equal(envelope.ok, true, JSON.stringify(envelope.errors));
    const receipt = await agentResult;
    const summary = { status: envelope.data.status, items: envelope.data.items.map(({ id, status }) => ({ id, status })),
      helper_source_bytes: lstatSync(binary).size,
      canonical_temporary_parent_is_tmp: rootNode("import fs from 'node:fs';console.log(fs.realpathSync('/tmp')==='/tmp');").stdout.trim() === 'true' };
    assert.equal(receipt.status, 0, `${receipt.stderr}\nCLI repair status: ${JSON.stringify(summary)}`);
    return envelope.data;
  } finally {
    if (child.exitCode === null) child.kill('SIGKILL');
    await output.catch(() => {});
    if (ready && agent.exitCode === null) {
      rootNode("import fs from 'node:fs'; const [pid,birth]=process.argv.slice(1); try{if(fs.readFileSync('/proc/'+pid+'/stat','utf8').split(')').at(-1).trim().split(/\\s+/)[19]===birth)process.kill(Number(pid),'SIGTERM');}catch(e){if(e.code!=='ENOENT')throw e;}", [String(ready.pid), ready.start]);
    }
    if (agentResult) await agentResult.catch(() => {});
  }
}

const active = run('/usr/bin/systemctl', ['is-active', 'systemd-timesyncd.service'], true).stdout.trim() === 'active';
const enabled = unitState();
const hadDirectory = existsSync(directory);
assert(!existsSync(targetFile), 'Refusing an existing product override in the disposable fixture');
assert(!existsSync(fixtureFile), 'Refusing an existing fixture file');
let original;
const directoryModes = ['/', '/etc', '/etc/systemd', '/run', '/run/systemd', '/usr', '/usr/local', '/usr/local/lib', '/usr/local/lib/systemd', '/usr/lib', '/usr/lib/systemd']
  .filter(path => existsSync(path)).map(path => {
    const meta = lstatSync(path);
    assert(meta.isDirectory() && meta.uid === 0, 'Refusing an unowned CI configuration ancestor');
    return { path, dev: meta.dev, ino: meta.ino, mode: meta.mode & 0o7777 };
  }).filter(node => node.mode & 0o022);
const permissions = String.raw`import fs from 'node:fs'; const [raw,restore]=process.argv.slice(1);for(const node of JSON.parse(raw)){const fd=fs.openSync(node.path,fs.constants.O_RDONLY|fs.constants.O_DIRECTORY|fs.constants.O_NOFOLLOW);try{const m=fs.fstatSync(fd);if(!m.isDirectory()||m.uid!==0||m.dev!==node.dev||m.ino!==node.ino)throw Error('Changed CI ancestor');const mode=m.mode&0o7777;const trusted=node.mode&~0o022;if(restore==='1'&&mode===node.mode)continue;if(mode!==(restore==='1'?trusted:node.mode))throw Error('Changed CI ancestor permissions');fs.fchmodSync(fd,restore==='1'?node.mode:trusted);}finally{fs.closeSync(fd);}}`;
let preparedModes = false;
try {
  if (!active) run('/usr/bin/sudo', ['-n', '/usr/bin/systemctl', 'start', 'systemd-timesyncd.service']);
  original = native();
  rootBus('SetRuntimeNTPServers', 'as', '0');
  const before = native();
  assert.deepEqual(before.RuntimeNTPServers, []);
  assert.throws(() => readlinkSync(`/proc/${before.pid}/exe`), error => ['EACCES', 'EPERM'].includes(error.code),
    'Ordinary-user preview must not depend on a privileged executable read');
  if (directoryModes.length) {
    assert.equal(cli('environment:repair-preview').data.items.find(item => item.id === 'ntp.server').status, 'unavailable');
    preparedModes = true;
    rootNode(permissions, [JSON.stringify(directoryModes), '0']);
  }
  const preview = cli('environment:repair-preview').data;
  assert.equal(preview.items.find(item => item.id === 'ntp.server').status, 'change');
  assert.equal(preview.items.find(item => item.id === 'ntp.server.persistent').status, 'change');
  if (!hadDirectory) assert.equal(preview.items.find(item => item.id === 'ntp.server.directory').status, 'change');
  assert(!existsSync(home)); assert(!existsSync(data)); assert(!existsSync(targetFile));
  assert.deepEqual(native(), before);

  const cancelled = await repair('ntp-cancel');
  assert.equal(cancelled.status, 'cancelled');
  assert(!existsSync(targetFile));
  if (!hadDirectory) assert(!existsSync(directory), 'Authorization cancellation created the config directory');
  assert.deepEqual(native(), before);
  const applied = await repair('ntp-allow');
  assert.equal(applied.status, 'applied', JSON.stringify(applied.items));
  assert(applied.items.every(item => item.status === 'verified'));
  assert(applied.verification_limits.some(limit => limit.includes('future instance')));
  assert.equal(readFileSync(targetFile, 'utf8'), generated(selected));
  const meta = lstatSync(targetFile);
  assert.equal(meta.mode & 0o7777, 0o644); assert.equal(meta.uid, 0); assert.equal(meta.nlink, 1);
  assert.deepEqual(native(), { ...before, RuntimeNTPServers: [selected] }, 'Repair changed unrelated instance state');
  assert(!existsSync(home));
  const database = readFileSync(join(data, 'native-manager.db'));
  const noopPreview = cli('environment:repair-preview').data;
  const noop = cli('environment:repair', ['--base', noopPreview.base, '--confirm']).data;
  assert.equal(noop.status, 'not_needed');
  assert(noop.verification_limits.some(limit => limit.includes('future instance')));
  assert.deepEqual(readFileSync(join(data, 'native-manager.db')), database);

  // A real inode policy is not an xattr. Preview must refuse it without
  // replacing the owned file or writing app state; restore only this exact inode.
  const flagsBefore = run('/usr/bin/lsattr', ['-d', targetFile]).stdout.trim().split(/\s+/)[0];
  const inodePolicy = flag => rootNode("import fs from 'node:fs';import{execFileSync}from'node:child_process';const[p,dev,ino,text,flag]=process.argv.slice(1);const s=fs.lstatSync(p);if(s.dev!==Number(dev)||s.ino!==Number(ino)||fs.readFileSync(p,'utf8')!==text)throw Error('Changed owned policy fixture');execFileSync('/usr/bin/chattr',[flag,p]);", [targetFile, String(meta.dev), String(meta.ino), generated(selected), flag]);
  inodePolicy('+d');
  try {
    assert(run('/usr/bin/lsattr', ['-d', targetFile]).stdout.trim().split(/\s+/)[0].includes('d'));
    assert.equal(cli('environment:repair-preview').data.items.find(item => item.id === 'ntp.server').status, 'unavailable');
    assert.equal(lstatSync(targetFile).ino, meta.ino);
    assert.equal(readFileSync(targetFile, 'utf8'), generated(selected));
    assert.deepEqual(readFileSync(join(data, 'native-manager.db')), database);
  } finally {
    inodePolicy('-d');
  }
  assert.equal(run('/usr/bin/lsattr', ['-d', targetFile]).stdout.trim().split(/\s+/)[0], flagsBefore);
  assert.equal(cli('environment:repair-preview').data.items.find(item => item.id === 'ntp.server').status, 'unchanged');

  rootNode("import fs from 'node:fs'; const [p,text,fixture,fixtureText]=process.argv.slice(1); if(fs.readFileSync(p,'utf8')!==text)throw Error('Changed owned file');fs.unlinkSync(p);fs.writeFileSync(fixture,fixtureText,{flag:'wx',mode:0o644});", [targetFile, generated(selected), fixtureFile, fixtureText]);
  const savedOnlyPreview = cli('environment:repair-preview').data;
  assert.equal(savedOnlyPreview.items.find(item => item.id === 'ntp.server').status, 'unchanged');
  assert.equal(savedOnlyPreview.items.find(item => item.id === 'ntp.server.persistent').status, 'change');
  const drifted = await repair('ntp-change');
  assert.equal(drifted.status, 'partial'); assert(!existsSync(targetFile));
  assert.equal(readFileSync(fixtureFile, 'utf8'), `${fixtureText}# CI authorization drift\n`);
  assert.deepEqual(native(), { ...before, RuntimeNTPServers: [selected] });
  rootNode("import fs from 'node:fs'; const [p,old,text]=process.argv.slice(1);if(fs.readFileSync(p,'utf8')!==old)throw Error('Changed CI drift file');fs.writeFileSync(p,text);", [fixtureFile, `${fixtureText}# CI authorization drift\n`, fixtureText]);
  for (const mode of ['eof', 'malformed', 'oversize', 'expiry', 'stale', 'image']) {
    const receipt = run('/usr/bin/sudo', ['-n', '/usr/bin/env', 'CI=true', 'GITHUB_ACTIONS=true', 'AM_ISOLATED_LINUX_NTP=1',
      process.execPath, resolve('scripts/verify-linux-ntp-helper.mjs'), binary, mode]);
    assert(receipt.stdout.includes('zero-write refusal passed.'));
    assert.deepEqual(native(), { ...before, RuntimeNTPServers: [selected] });
    assert(!existsSync(targetFile));
  }
  // A UID-0 helper holds the same fixed host lock while an ordinary UID's
  // separately authorized helper tries to enter. No per-user lock can mask this.
  const holding = spawn('/usr/bin/sudo', ['-n', '/usr/bin/env', 'CI=true', 'GITHUB_ACTIONS=true', 'AM_ISOLATED_LINUX_NTP=1',
    process.execPath, resolve('scripts/verify-linux-ntp-helper.mjs'), binary, 'lock'], { stdio: ['pipe', 'pipe', 'pipe'] });
  const held = collect(holding);
  await new Promise((resolve, reject) => {
    let text = '';
    const timeout = setTimeout(() => reject(Error('Cross-UID lock fixture was not ready')), 10_000);
    holding.stdout.on('data', chunk => { text += chunk; if (text.includes('\n')) { clearTimeout(timeout); try { assert.equal(JSON.parse(text.split('\n')[0]).ready, true); resolve(); } catch (error) { reject(error); } } });
    holding.on('close', () => { clearTimeout(timeout); reject(Error('Lock fixture exited early')); });
  });
  try {
    assert.equal((await repair('ntp-allow')).status, 'partial');
    assert(!existsSync(targetFile));
    assert.deepEqual(native(), { ...before, RuntimeNTPServers: [selected] });
  } finally {
    holding.stdin.end('release\n');
    const receipt = await held;
    assert.equal(receipt.status, 0, receipt.stderr);
    assert(receipt.stdout.includes('zero-write refusal passed.'));
  }
  const savedOnly = await repair('ntp-allow');
  assert.equal(savedOnly.status, 'applied');
  assert.equal(savedOnly.items.find(item => item.id === 'ntp.server').status, 'unchanged');
  assert.equal(savedOnly.items.find(item => item.id === 'ntp.server.persistent').status, 'verified');
  assert.deepEqual(native(), { ...before, RuntimeNTPServers: [selected] });

  const stalePreview = cli('environment:repair-preview').data;
  const staleDatabase = readFileSync(join(data, 'native-manager.db'));
  rootNode("import fs from 'node:fs'; const [p,old,text]=process.argv.slice(1);if(fs.readFileSync(p,'utf8')!==old)throw Error('Changed owned file');fs.writeFileSync(p,text);", [targetFile, generated(selected), generated('127.0.0.2')]);
  const stale = cli('environment:repair', ['--base', stalePreview.base, '--confirm'], true);
  assert.equal(stale.ok, false); assert(stale.errors.some(error => error.message.includes('changed after preview')));
  assert.deepEqual(readFileSync(join(data, 'native-manager.db')), staleDatabase);
  assert.deepEqual(native(), { ...before, RuntimeNTPServers: [selected] });
  rootNode("import fs from 'node:fs'; const [p,old,text]=process.argv.slice(1);if(fs.readFileSync(p,'utf8')!==old)throw Error('Changed owned file');fs.writeFileSync(p,text);", [targetFile, generated('127.0.0.2'), generated(selected)]);

  // Only the disposable fixture restarts the service. Empty runtime is essential:
  // retaining the old runtime override would not prove loading of the saved file.
  run('/usr/bin/sudo', ['-n', '/usr/bin/systemctl', 'restart', 'systemd-timesyncd.service']);
  const restarted = native();
  assert.notEqual(restarted.owner, before.owner); assert.notEqual(restarted.pid, before.pid);
  assert.deepEqual(restarted.RuntimeNTPServers, []);
  assert.deepEqual(restarted.SystemNTPServers, [selected]);
  assert.deepEqual(restarted.FallbackNTPServers, ['198.51.100.2']);
  assert.equal(restarted.enabled, enabled);
  await lostReply({ run, rootNode, cli, repair, native, generated, selected, targetFile, env });
  console.log('Native Linux NTP: ordinary-user preview, actual Polkit challenge/cancellation, file+runtime readback, saved-only repair, no-op, authorization drift, stale refusal and external-restart loading passed. Future loading on other devices, desktop dialogs and clock accuracy remain unverified.');
} finally {
  const errors = [];
  for (const [path, admitted] of [[targetFile, [generated(selected), generated('127.0.0.2')]], [fixtureFile, [fixtureText, `${fixtureText}# CI authorization drift\n`]], [rule, policy ? [policy] : []]]) {
    try {
      rootNode("import fs from 'node:fs';const [p,raw]=process.argv.slice(1);if(fs.existsSync(p)){const m=fs.lstatSync(p);if(!m.isFile()||m.uid!==0||m.nlink!==1||!JSON.parse(raw).includes(fs.readFileSync(p,'utf8')))throw Error('Refusing changed fixture cleanup');fs.unlinkSync(p);}", [path, JSON.stringify(admitted)]);
    } catch (error) { errors.push(error); }
  }
  if (!hadDirectory) {
    try { rootNode("import fs from 'node:fs';const p=process.argv[1];if(fs.existsSync(p)){if(fs.readdirSync(p).length)throw Error('Unexpected remaining NTP entries');fs.rmdirSync(p);}", [directory]); } catch (error) { errors.push(error); }
  }
  try {
    if (original && !errors.length) {
      run('/usr/bin/sudo', ['-n', '/usr/bin/systemctl', 'restart', 'systemd-timesyncd.service']);
      rootBus('SetRuntimeNTPServers', 'as', String(original.RuntimeNTPServers.length), ...original.RuntimeNTPServers);
      const restored = native();
      for (const key of ['RuntimeNTPServers', 'SystemNTPServers', 'LinkNTPServers', 'FallbackNTPServers', 'RootDistanceMaxUSec', 'PollIntervalMinUSec', 'PollIntervalMaxUSec', 'enabled']) assert.deepEqual(restored[key], original[key], `Native restoration differs: ${key}`);
      if (!active) run('/usr/bin/sudo', ['-n', '/usr/bin/systemctl', 'stop', 'systemd-timesyncd.service']);
    }
    assert.equal(unitState(), enabled);
    assert(!existsSync(home));
  } catch (error) { errors.push(error); }
  if (preparedModes) {
    try {
      rootNode(permissions, [JSON.stringify(directoryModes), '1']);
      for (const node of directoryModes) assert.equal(lstatSync(node.path).mode & 0o7777, node.mode);
    } catch (error) { errors.push(error); }
  }
  rmSync(root, { recursive: true });
  if (errors.length) throw new AggregateError(errors, 'Independent NTP fixture restoration failed');
  console.log('Native Linux NTP: original configuration, server lists, service state and unit enablement restored independently.');
}
