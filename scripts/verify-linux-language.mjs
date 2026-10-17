// Real distro AccountsService and Polkit on a private disposable CI bus.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { userInfo } from 'node:os';

assert.equal(process.platform, 'linux');
assert.equal(process.env.CI, 'true');
assert.equal(process.env.RUNNER_ENVIRONMENT, 'github-hosted');
assert.equal(process.env.AM_ISOLATED_LINUX_LANGUAGE, '1');
assert.notEqual(process.getuid(), 0);
assert.equal(process.env.DBUS_SYSTEM_BUS_ADDRESS, process.env.DBUS_SESSION_BUS_ADDRESS);
const root = process.env.ROOTDIR;
assert(root && root.startsWith(process.env.RUNNER_TEMP + '/gnome-language.'));
const pam = join(userInfo().homedir, '.pam_environment');
const originalPam = existsSync(pam) ? readFileSync(pam) : null;
const binary = resolve('engine-rs/target/debug/agent-manager-engine');
const path = `/org/freedesktop/Accounts/User${process.getuid()}`;
const spec = JSON.stringify({ languages: ['fr-FR'] });
function run(command, args, failure = false) {
  const result = spawnSync(command, args, { encoding: 'utf8', timeout: 40_000, maxBuffer: 1_048_576 });
  assert.ifError(result.error);
  if (!failure) assert.equal(result.status, 0, result.stderr);
  return result;
}
function cli(command, flags = [], failure = false, profile = spec) {
  const result = run(binary, [command, '--spec', profile, ...flags, '--json'], failure);
  const envelope = JSON.parse(result.stdout);
  if (!failure) assert.equal(envelope.ok, true, JSON.stringify(envelope.errors));
  return envelope;
}
function bus(method, signature, ...args) {
  const raw = run('/usr/bin/busctl', ['--system', '--auto-start=no', '--json=short', 'call',
    'org.freedesktop.Accounts', path, 'org.freedesktop.Accounts.User', method, signature, ...args]).stdout.trim();
  return raw ? JSON.parse(raw) : null;
}
function property(key) {
  return JSON.parse(run('/usr/bin/busctl', ['--system', '--auto-start=no', '--json=short', 'call',
    'org.freedesktop.Accounts', path, 'org.freedesktop.DBus.Properties', 'Get', 'ss',
    'org.freedesktop.Accounts.User', key]).stdout).data[0].data;
}
function set(languages) { bus('SetLanguages', 'as', String(languages.length), ...languages); }
function gsettings(...args) { return run('/usr/bin/gsettings', args).stdout.trim(); }
function collected(child) {
  return new Promise((resolve, reject) => {
    let out = '', error = '';
    child.stdout.on('data', data => { out += data; });
    child.stderr.on('data', data => { error += data; });
    child.once('error', reject);
    child.once('exit', code => resolve({ status: code, stdout: out, stderr: error }));
  });
}
const daemon = ['/usr/libexec/accounts-daemon', '/usr/lib/accountsservice/accounts-daemon'].find(existsSync);
assert(daemon, 'Distro AccountsService executable is unavailable');
const user = run('/usr/bin/id', ['-un']).stdout.trim();
assert.match(user, /^[A-Za-z0-9_-]+$/);
const cacheDir = join(root, 'var/lib/AccountsService/users');
mkdirSync(cacheDir, { recursive: true });
mkdirSync(join(root, 'var/lib/AccountsService/icons'), { recursive: true });
mkdirSync(join(root, 'etc'), { recursive: true });
writeFileSync(join(cacheDir, user), '[User]\nLanguage=en_US.UTF-8\nLanguages=en_US.UTF-8;de_DE.UTF-8;\n', { mode: 0o600 });
const action = 'org.freedesktop.accounts.change-own-user-data';
const polkit = ['/usr/lib/polkit-1/polkitd', '/usr/libexec/polkitd'].find(existsSync);
assert(polkit, 'Distro Polkit executable is unavailable');
const serviceUid = process.env.AM_LANGUAGE_SERVICE_UID ?? '';
const serviceGid = process.env.AM_LANGUAGE_SERVICE_GID ?? '';
assert.match(serviceUid, /^[1-9][0-9]*$/); assert.match(serviceGid, /^[1-9][0-9]*$/);
const rule = `/etc/polkit-1/rules.d/49-agent-manager-language-${process.pid}.rules`;
const policy = `polkit.addRule(function(action, subject) { if (action.id === "${action}" && subject.user === ${JSON.stringify(user)}) return polkit.Result.YES; });\n`;
function rootNode(source, args) { return run('/usr/bin/sudo', ['-n', process.execPath, '--input-type=module', '-e', source, ...args]); }
rootNode("import fs from 'node:fs';const [p,t]=process.argv.slice(1);const fd=fs.openSync(p,'wx',0o644);fs.writeFileSync(fd,t);fs.closeSync(fd);", [rule, policy]);
let installedPolicy = policy;
function replacePolicy(next) {
  rootNode("import fs from 'node:fs';const [p,old,t]=process.argv.slice(1);if(fs.readFileSync(p,'utf8')!==old)throw Error('Foreign policy');fs.writeFileSync(p,t);", [rule, installedPolicy, next]);
  installedPolicy = next;
}
// An owned root supervisor kills only its child on EOF or deadline, including failed startup.
const supervisor = `import {spawn} from 'node:child_process';
const child=spawn('/usr/bin/setpriv',['--reuid',process.argv[2],'--regid',process.argv[3],'--init-groups',process.argv[1]],{cwd:'/',stdio:['ignore','pipe','pipe']});
child.stdout.pipe(process.stdout);child.stderr.pipe(process.stderr);
const stop=()=>{if(child.exitCode===null)child.kill('SIGTERM');};
const timer=setTimeout(stop,180000);process.stdin.resume();process.stdin.on('end',stop);
child.on('error',error=>{console.error(error);process.exit(1);});
child.on('exit',code=>{clearTimeout(timer);process.exit(code??0);});`;
const authority = spawn('/usr/bin/sudo', ['-n', '/usr/bin/env', `DBUS_SYSTEM_BUS_ADDRESS=${process.env.DBUS_SYSTEM_BUS_ADDRESS}`, process.execPath, '--input-type=module', '-e', supervisor, polkit, serviceUid, serviceGid], { stdio: ['pipe', 'pipe', 'pipe'] });
const authorityResult = collected(authority);
let authorityStartup = '';
for (const stream of [authority.stdout, authority.stderr]) stream.on('data', data => { authorityStartup = (authorityStartup + data).slice(-16_384); });
let authorityIdentity;
function ownerIdentity() {
  const owner = JSON.parse(run('/usr/bin/busctl', ['--system', '--auto-start=no', '--json=short', 'call', 'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'GetNameOwner', 's', 'org.freedesktop.PolicyKit1']).stdout).data[0];
  const pid = JSON.parse(run('/usr/bin/busctl', ['--system', '--auto-start=no', '--json=short', 'call', 'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'GetConnectionUnixProcessID', 's', owner]).stdout).data[0];
  const start = readFileSync(`/proc/${pid}/stat`, 'utf8').split(')').at(-1).trim().split(/\s+/)[19];
  return { owner, pid, start };
}
async function authenticate(mode) {
  const preview = cli('environment:repair-preview').data;
  console.log('LANGUAGE_PREVIEW_RECEIPT ' + JSON.stringify(preview));
  const child = spawn('/bin/sh', ['-c', 'kill -STOP $$; exec "$@"', 'am-owned-cli', binary,
    'environment:repair', '--spec', spec, '--base', preview.base, '--confirm', '--json'],
    { env: process.env, stdio: ['ignore', 'pipe', 'pipe'] });
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
    const challenge = `polkit.addRule(function(action, subject) { if (action.id === "${action}" && subject.user === ${JSON.stringify(user)}) return subject.pid === ${child.pid} ? polkit.Result.AUTH_SELF_KEEP : polkit.Result.YES; });\n`;
    rootNode("import fs from 'node:fs'; const [p,old,t]=process.argv.slice(1); if(fs.readFileSync(p,'utf8')!==old) throw Error('Foreign policy'); fs.writeFileSync(p,t);", [rule, current, challenge]);
    installedPolicy = challenge;
    const subject = `${child.pid},${fields[19]},${process.getuid()}`;
    const policyBy = Date.now() + 10_000;
    let check;
    do {
      check = run('/usr/bin/pkcheck', ['--action-id', action, '--process', subject], true).status;
      if (check === 2) break;
      assert([0, 1].includes(check), 'CI challenge query failed');
      assert(Date.now() < policyBy, 'Exact-process authentication challenge was not loaded');
      await delay(100);
    } while (true);
    const example = resolve('engine-rs/target/debug/examples/linux_polkit_fixture');
    agent = spawn('/usr/bin/sudo', ['-n', '/usr/bin/env', 'CI=true', 'AM_ISOLATED_LINUX_LANGUAGE=1', `DBUS_SYSTEM_BUS_ADDRESS=${process.env.DBUS_SYSTEM_BUS_ADDRESS}`, `AM_LANGUAGE_PRIVATE_BUS=${process.env.DBUS_SYSTEM_BUS_ADDRESS}`, example,
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
    const envelope = JSON.parse(result.stdout);
    console.log('LANGUAGE_AUTH_CLI ' + JSON.stringify({ mode, ok: envelope.ok, status: envelope.data?.status, items: envelope.data?.items, errors: envelope.errors }));
    assert.equal(envelope.ok, true, JSON.stringify(envelope.errors));
    const receipt = await agentResult;
    assert.equal(receipt.status, 0, `CI authentication protocol failed (${mode}): ${receipt.stderr}`);
    return envelope.data;
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
let service, serviceResult;
async function start() {
  service = spawn(daemon, [], { stdio: ['ignore', 'pipe', 'pipe'] });
  serviceResult = collected(service);
  const by = Date.now() + 10_000;
  while (true) {
    const result = run('/usr/bin/busctl', ['--system', '--auto-start=no', 'get-property',
      'org.freedesktop.Accounts', path, 'org.freedesktop.Accounts.User', 'Language'], true);
    if (result.status === 0) break;
    assert(Date.now() < by, 'Private AccountsService did not become ready');
    await delay(50);
  }
}
try {
  const authorityBy = Date.now() + 10_000;
  while (true) {
    const ready = run('/usr/bin/busctl', ['--system', '--auto-start=no', 'call', 'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'GetNameOwner', 's', 'org.freedesktop.PolicyKit1'], true);
    if (ready.status === 0) break;
    assert(Date.now() < authorityBy, 'Private Polkit did not become ready: ' + authorityStartup);
    await delay(50);
  }
  authorityIdentity = ownerIdentity();
  assert(![process.getuid(), Number(serviceUid)].includes(65534));
  const outsider = run('/usr/bin/sudo', ['-n', '/usr/bin/setpriv', '--reuid', '65534', '--regid', '65534', '--clear-groups',
    '/usr/bin/busctl', `--address=${process.env.DBUS_SYSTEM_BUS_ADDRESS}`, '--auto-start=no', 'call',
    'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'GetId'], true);
  assert.notEqual(outsider.status, 0, 'Private bus admitted an unselected UID');
  await start();
  assert.equal(property('Language'), 'en_US.UTF-8');
  gsettings('set', 'org.gnome.system.locale', 'region', "'en_GB.UTF-8'");
  const formats = gsettings('get', 'org.gnome.system.locale', 'region');
  const originalCache = readFileSync(join(cacheDir, user));
  const preview = cli('environment:repair-preview').data;
  assert.equal(preview.items[0].status, 'change');
  assert(!existsSync(process.env.HOME));
  assert(!existsSync(process.env.AGENT_MANAGER_DATA_DIR));
  assert.deepEqual(readFileSync(join(cacheDir, user)), originalCache);
  const applied = cli('environment:repair', ['--base', preview.base, '--confirm']).data;
  console.log('LANGUAGE_APPLIED_RECEIPT ' + JSON.stringify(applied));
  assert.equal(applied.status, 'applied', JSON.stringify(applied));
  assert.equal(applied.items[0].status, 'verified');
  assert.equal(applied.relogin_required, true);
  assert(applied.verification_limits.some(value => value.includes('Persistent storage')));
  assert.equal(property('Language').replace('utf8', 'UTF-8'), 'fr_FR.UTF-8');
  assert.deepEqual(property('Languages').slice(1), ['de_DE.UTF-8']);
  assert.equal(gsettings('get', 'org.gnome.system.locale', 'region'), formats);
  const savedCache = readFileSync(join(cacheDir, user));
  service.kill('SIGTERM'); await serviceResult;
  await start();
  assert.equal(property('Language').replace('utf8', 'UTF-8'), 'fr_FR.UTF-8');
  assert.deepEqual(property('Languages').slice(1), ['de_DE.UTF-8']);
  const noop = cli('environment:repair-preview').data;
  assert.equal(cli('environment:repair', ['--base', noop.base, '--confirm']).data.status, 'not_needed');
  assert.deepEqual(readFileSync(join(cacheDir, user)), savedCache);
  set(['en_US.UTF-8', 'de_DE.UTF-8']);
  for (const value of ['cancel', 'change', 'allow']) {
    const result = await authenticate(`language-${value}`);
    assert.equal(result.status, value === 'cancel' ? 'cancelled' : value === 'change' ? 'partial' : 'applied', JSON.stringify(result));
    assert.equal(result.items[0].status, value === 'cancel' ? 'not_applied' : value === 'change' ? 'stale' : 'verified', JSON.stringify(result));
    assert.equal(property('Language').replace('utf8', 'UTF-8'), value === 'allow' ? 'fr_FR.UTF-8' : 'en_US.UTF-8');
    assert.deepEqual(property('Languages').slice(1), [value === 'change' ? 'fr_FR.UTF-8' : 'de_DE.UTF-8']);
    assert.equal(gsettings('get', 'org.gnome.system.locale', 'region'), formats);
    replacePolicy(policy);
    set(['en_US.UTF-8', 'de_DE.UTF-8']);
  }
  for (const profile of [JSON.stringify({ languages: ['en-US', 'fr-FR'] }), JSON.stringify({ languages: ['zz-ZZ'] })]) {
    assert.equal(cli('environment:repair-preview', [], false, profile).data.items[0].status, 'unavailable');
  }
  gsettings('reset', 'org.gnome.system.locale', 'region');
  assert.equal(cli('environment:repair-preview').data.items[0].status, 'unavailable');
  assert(!existsSync(process.env.HOME));
  assert.deepEqual(existsSync(pam) ? readFileSync(pam) : null, originalPam, 'Provider changed caller PAM overrides');
  console.log('Private native AccountsService and Polkit: primary preference, metadata/formats preservation, restart persistence, no-op, genuine authorization cancellation/change/same-process dispatch and unavailable targets passed; desktop adoption remains unverified.');
} finally {
  if (service && service.exitCode === null) { service.kill('SIGTERM'); await serviceResult; }
  if (authorityIdentity) assert.deepEqual(ownerIdentity(), authorityIdentity, 'Private authority identity changed');
  authority.stdin.end();
  await authorityResult;
  rootNode("import fs from 'node:fs';const [p,t]=process.argv.slice(1);if(fs.readFileSync(p,'utf8')!==t)throw Error('Foreign policy');fs.unlinkSync(p);", [rule, installedPolicy]);
}
