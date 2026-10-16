// Root-side protocol fixture, invoked only by verify-linux-ntp on a disposable runner.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { chmodSync, existsSync, lstatSync, readFileSync, writeFileSync } from 'node:fs';
import { setTimeout as delay } from 'node:timers/promises';

assert.equal(process.platform, 'linux');
assert.equal(process.getuid(), 0);
assert.equal(process.env.CI, 'true');
assert.equal(process.env.GITHUB_ACTIONS, 'true');
assert.equal(process.env.AM_ISOLATED_LINUX_NTP, '1');
const [binary, mode] = process.argv.slice(2);
assert(binary?.endsWith('/engine-rs/target/release/agent-manager-engine'));
assert(['eof', 'malformed', 'oversize', 'expiry', 'stale', 'image', 'lock'].includes(mode));
const path = '/etc/systemd/timesyncd.conf.d/90-agent-manager.conf';
const earlier = '/etc/systemd/timesyncd.conf.d/80-agent-manager-ci.conf';
const executable = '/usr/lib/systemd/systemd-timesyncd';
const originalMode = lstatSync(executable).mode & 0o7777;
const originalEarlier = readFileSync(earlier, 'utf8');
assert(!existsSync(path));
const birth = readFileSync('/proc/self/stat', 'utf8').split(')').at(-1).trim().split(/\s+/)[19];
const deadline = Math.floor(Number(readFileSync('/proc/uptime', 'utf8').split(' ')[0]) * 1000) + (mode === 'expiry' ? 5000 : 120_000);
const child = spawn(binary, ['--environment-ntp-helper', '127.0.0.1', String(process.pid), birth, '0', String(deadline), '--confirm'],
  { env: { PATH: '/usr/bin:/bin', LC_ALL: 'C' }, stdio: ['pipe', 'pipe', 'pipe'] });
let output = '', stderr = '';
const frames = [];
child.stdout.on('data', chunk => {
  output += chunk;
  assert(output.length <= 32768);
  while (output.includes('\n')) {
    const at = output.indexOf('\n');
    frames.push(JSON.parse(output.slice(0, at)));
    output = output.slice(at + 1);
  }
});
child.stderr.on('data', chunk => { stderr += chunk; assert(stderr.length <= 8192); });
const closed = new Promise((resolve, reject) => {
  child.on('error', reject);
  child.on('close', code => resolve(code));
});
async function waitFor(condition, ms = 10_000) {
  const until = Date.now() + ms;
  while (!condition()) {
    assert(Date.now() < until, 'Owned helper protocol fixture timed out');
    await delay(20);
  }
}
let changedImage = false;
let changedEarlier = false;
try {
  await waitFor(() => frames.length || child.exitCode !== null);
  assert.equal(frames[0]?.message, 'ready', `Native helper failed before READY: ${child.exitCode}, ${stderr}`);
  assert.equal(frames[0].version, 1);
  assert.match(frames[0].base, /^[0-9a-f]{64}$/);
  assert(!existsSync(path), 'READY was not zero-write');
  if (mode === 'eof') child.stdin.end();
  else if (mode === 'malformed') child.stdin.write('{"message":"commit","version":99,"base":"' + frames[0].base + '"}\n');
  else if (mode === 'oversize') child.stdin.write('x'.repeat(8193));
  else if (mode === 'expiry') await delay(6000);
  else if (mode === 'stale') {
    writeFileSync(earlier, `${originalEarlier}# CI READY drift\n`);
    changedEarlier = true;
    child.stdin.write(JSON.stringify({ message: 'commit', version: 1, base: frames[0].base }) + '\n');
  } else if (mode === 'image') {
    chmodSync(executable, originalMode === 0o755 ? 0o555 : 0o755);
    changedImage = true;
    child.stdin.write(JSON.stringify({ message: 'commit', version: 1, base: frames[0].base }) + '\n');
  } else if (mode === 'lock') {
    process.stdout.write(JSON.stringify({ ready: true, pid: process.pid, birth }) + '\n');
    await new Promise(resolve => process.stdin.once('data', resolve));
    child.stdin.end();
  }
  await waitFor(() => child.exitCode !== null);
  assert.equal(await closed, 74);
  assert(!existsSync(path), 'Rejected helper request published a file');
  assert.equal(frames.length, 1, 'Rejected helper request advanced beyond READY');
  console.log(`Native NTP helper ${mode}: zero-write refusal passed.`);
} finally {
  child.stdin.destroy();
  if (child.exitCode === null) child.kill('SIGKILL');
  await closed.catch(() => {});
  if (changedImage) {
    assert.equal(lstatSync(executable).mode & 0o7777, originalMode === 0o755 ? 0o555 : 0o755);
    chmodSync(executable, originalMode);
    assert.equal(lstatSync(executable).mode & 0o7777, originalMode);
  }
  if (changedEarlier) {
    assert.equal(readFileSync(earlier, 'utf8'), `${originalEarlier}# CI READY drift\n`);
    writeFileSync(earlier, originalEarlier);
    assert.equal(readFileSync(earlier, 'utf8'), originalEarlier);
  }
}
