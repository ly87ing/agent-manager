// Disposable native KDE opener acceptance; never run on a personal desktop.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, lstatSync, readFileSync, readdirSync, readlinkSync, realpathSync, statSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { verifyPreferences } from './verify-linux-kde-preferences.mjs';

assert.equal(process.platform, 'linux');
assert.equal(process.env.CI, 'true');
assert.equal(process.env.RUNNER_ENVIRONMENT, 'github-hosted');
assert.equal(process.env.AM_ISOLATED_LINUX_SETTINGS, '1');
assert(process.getuid() !== 0);
assert(process.env.DISPLAY && process.env.DBUS_SESSION_BUS_ADDRESS);

const engine = realpathSync(resolve(process.argv[2] ?? 'engine-rs/target/debug/agent-manager-engine'));
const allowed = new Set(['/usr/bin/systemsettings', '/usr/bin/kcmshell5']
  .filter(path => existsSync(path)).map(path => realpathSync(path)));
const root = realpathSync(process.env.AM_LINUX_SETTINGS_ROOT);
assert.equal(dirname(root), realpathSync(process.env.RUNNER_TEMP));
assert(basename(root).startsWith('kde-settings.'));
assert.equal(lstatSync(root).uid, process.getuid());
assert.equal(lstatSync(root).mode & 0o777, 0o700);
for (const [name, directory] of Object.entries({ HOME: 'home', USERPROFILE: 'home',
  XDG_CONFIG_HOME: 'config', XDG_CACHE_HOME: 'cache', XDG_DATA_HOME: 'data', XDG_RUNTIME_DIR: 'runtime' })) {
  assert.equal(process.env[name], join(root, directory), 'The private bus must inherit isolation before startup');
  assert(lstatSync(join(root, directory)).isDirectory());
}
const data = join(root, 'manager-data');
assert.equal(process.env.AGENT_MANAGER_DATA_DIR, data);
const env = { ...process.env };
const owned = new Map();
const sessions = new Set();

function command(program, args, allowFailure = false) {
  const result = spawnSync(program, args, { env, detached: program === engine,
    encoding: 'utf8', timeout: 20_000, maxBuffer: 1024 * 1024 });
  if (program === engine && result.pid) sessions.add(result.pid);
  if (!allowFailure) {
    assert.equal(result.status, 0, `${program}: ${result.stderr || result.error || result.stdout}`);
  }
  return result;
}
function identity(pid) {
  try {
    const stat = readFileSync(`/proc/${pid}/stat`, 'utf8');
    const fields = stat.slice(stat.lastIndexOf(')') + 1).trim().split(/\s+/);
    return { exe: readlinkSync(`/proc/${pid}/exe`), start: fields[19],
      group: Number(fields[2]), session: Number(fields[3]), uid: statSync(`/proc/${pid}`).uid };
  } catch {
    return null;
  }
}
function same(pid, expected) {
  const actual = identity(pid);
  return actual?.exe === expected.exe && actual.start === expected.start
    && actual.uid === expected.uid && actual.session === expected.session && actual.group === expected.group;
}
function signal(pid, expected, name) {
  if (!same(pid, expected)) return;
  try {
    process.kill(pid, name);
  } catch (error) {
    if (error.code !== 'ESRCH') throw error;
  }
}
async function windows() {
  let tree;
  for (let attempt = 0; attempt < 5; attempt++) {
    const result = command('/usr/bin/xwininfo', ['-root', '-tree'], true);
    if (result.status === 0) {
      tree = result.stdout;
      break;
    }
    // A child can disappear between QueryTree and its geometry query.
    const errors = [...(result.stderr ?? '').matchAll(/X Error: (\d+): ([^\r\n]+)/g)];
    const vanished = result.status === 1 && !result.error && errors.length > 0
      && errors.every(([, code, name]) => code === '3' && name === 'Bad Window'
        || code === '9' && name === 'Bad Drawable')
      && result.stderr.includes("error: Can't query window tree.");
    assert(vanished && attempt < 4, `Window tree query failed: ${result.stderr || result.error}`);
    console.log('A window disappeared during discovery; querying the tree again');
    await delay(50);
  }
  const ids = [...tree.matchAll(/^\s+(0x[\da-f]+)\s/gim)].map(match => match[1]);
  const found = [];
  for (const id of ids) {
    const properties = command('/usr/bin/xprop', ['-id', id, '_NET_WM_PID', 'WM_CLASS'], true);
    const pid = Number(properties.stdout?.match(/_NET_WM_PID[^=]*=\s*(\d+)/)?.[1]);
    const actual = identity(pid);
    if (actual && allowed.has(actual.exe)) {
      found.push({ pid, ...actual });
    }
  }
  return found;
}
async function stopWindows() {
  let discoveryError;
  try {
    // The CLI may fail after spawning, before any window can be observed.
    for (const name of sessions.size ? readdirSync('/proc') : []) {
      if (!/^\d+$/.test(name)) continue;
      const pid = Number(name);
      const actual = identity(pid);
      if (actual?.uid === process.getuid() && allowed.has(actual.exe)
          && sessions.has(actual.session) && actual.group === actual.session) {
        owned.set(pid, actual);
      }
    }
  } catch (error) {
    discoveryError = error;
  }
  for (const [pid, expected] of owned) {
    signal(pid, expected, 'SIGTERM');
  }
  const deadline = Date.now() + 5_000;
  while ([...owned].some(([pid, expected]) => same(pid, expected)) && Date.now() < deadline) {
    await delay(50);
  }
  for (const [pid, expected] of owned) {
    signal(pid, expected, 'SIGKILL');
  }
  const killedBy = Date.now() + 5_000;
  while ([...owned].some(([pid, expected]) => same(pid, expected)) && Date.now() < killedBy) {
    await delay(50);
  }
  assert([...owned].every(([pid, expected]) => !same(pid, expected)), 'Owned settings process survived cleanup');
  owned.clear();
  sessions.clear();
  if (discoveryError) throw discoveryError;
}

try {
  verifyPreferences(engine, env, root);
  assert.equal((await windows()).length, 0, 'The private display must start without settings windows');
  const listed = command('/usr/bin/systemsettings', ['--list']).stdout;
  const modules = new Set(listed.split('\n').map(line => line.split(' - ')[0].trim()));
  const merged = modules.has('kcm_regionandlang');
  assert(modules.has('kcm_clock'), 'The native clock module must be installed');
  assert(merged || (modules.has('kcm_translations') && modules.has('kcm_formats')),
    'The native language and formats modules must be installed');
  console.log(command('/usr/bin/dpkg-query', ['-W', '-f=${Package} ${Version}\n',
    'systemsettings', 'plasma-workspace', 'plasma-desktop', 'kde-cli-tools']).stdout.trim());
  const zone = realpathSync('/etc/localtime');
  for (const pane of ['language-region', 'date-time']) {
    const expected = realpathSync(pane === 'language-region' && !merged ? '/usr/bin/kcmshell5' : '/usr/bin/systemsettings');
    const result = command(engine, ['environment:open-settings', '--pane', pane, '--json']);
    const response = JSON.parse(result.stdout);
    assert.equal(response.ok, true, JSON.stringify(response));
    assert.equal(response.data.pane, pane);
    const deadline = Date.now() + 20_000;
    let visible;
    do {
      visible = await windows();
      for (const window of visible) {
        if (window.uid === process.getuid() && sessions.has(window.session) && window.group === window.session) {
          owned.set(window.pid, window);
        }
      }
      if (visible.some(window => window.exe === expected && owned.has(window.pid))) break;
      await delay(100);
    } while (Date.now() < deadline);
    const opened = visible.find(window => window.exe === expected && owned.has(window.pid));
    assert(opened, `No native settings window for ${pane}`);
    const args = readFileSync(`/proc/${opened.pid}/cmdline`, 'utf8').split('\0').filter(Boolean);
    assert.deepEqual(args.slice(1), pane === 'date-time' ? ['kcm_clock'] :
      merged ? ['kcm_regionandlang'] : ['kcm_translations', 'kcm_formats']);
    assert.equal(existsSync(data), false, 'Opening settings created manager state');
    assert.equal(realpathSync('/etc/localtime'), zone, 'Opening settings changed the timezone');
    console.log(`KDE ${merged ? 'merged' : 'separate'} modules: ${pane} opened an owned native window`);
    await stopWindows();
  }
} finally {
  await stopWindows();
}
