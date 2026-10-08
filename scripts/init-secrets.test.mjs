import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, statSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

const script = fileURLToPath(new URL('./init-secrets.sh', import.meta.url));
test('克隆用户仓库到外部，拒绝覆盖、项目内目录和缺少地址', t => {
  const root = mkdtempSync(join(tmpdir(), 'secrets-clone-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const source = join(root, 'source');
  const target = join(root, 'private');
  mkdirSync(source);
  const git = args => {
    const r = spawnSync('git', args, { cwd: source, encoding: 'utf8' });
    assert.equal(r.status, 0, r.stderr);
  };
  git(['init', '--quiet']);
  writeFileSync(join(source, 'mcp.env'), 'YOUR_TOKEN=YOUR_VALUE\n');
  git(['add', 'mcp.env']);
  git(['-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', '-c', 'core.hooksPath=/dev/null', 'commit', '--quiet', '-m', 'fixture']);
  const run = args => spawnSync('bash', [script, ...args], { encoding: 'utf8', env: { ...process.env, SECRETS_REPO_URL: '' } });
  assert.equal(run([source, target]).status, 0);
  assert.equal(readFileSync(join(target, 'mcp.env'), 'utf8'), 'YOUR_TOKEN=YOUR_VALUE\n');
  assert.equal(statSync(join(target, 'mcp.env')).mode & 0o777, 0o600);
  writeFileSync(join(target, 'mcp.env'), 'YOUR_TOKEN=YOUR_LOCAL_CHANGE\n');
  assert.notEqual(run([source, target]).status, 0);
  assert.equal(readFileSync(join(target, 'mcp.env'), 'utf8'), 'YOUR_TOKEN=YOUR_LOCAL_CHANGE\n');
  assert.notEqual(run([]).status, 0);
  const inside = fileURLToPath(new URL('../YOUR_TEST_SECRETS', import.meta.url));
  assert.notEqual(run([source, inside]).status, 0);
  assert.throws(() => statSync(inside), { code: 'ENOENT' });
});
