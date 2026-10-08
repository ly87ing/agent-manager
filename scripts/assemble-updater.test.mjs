import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

const script = fileURLToPath(new URL('./assemble-updater.mjs', import.meta.url));
const bundles = [
  'Agent Manager_aarch64.app.tar.gz', 'Agent Manager_x64.app.tar.gz',
  'Agent-Manager_amd64.AppImage', 'Agent-Manager_x64.msi', 'Agent-Manager_x64-setup.exe',
];
function fixture(t) {
  const cwd = mkdtempSync(join(tmpdir(), 'updater-manifest-'));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  const dir = join(cwd, 'updater-assets');
  mkdirSync(dir);
  for (const name of bundles) {
    writeFileSync(join(dir, name), 'test bundle');
    writeFileSync(join(dir, `${name}.sig`), 'test signature\n');
  }
  return { cwd, dir, run: () => spawnSync(process.execPath, [script], {
    cwd, encoding: 'utf8', env: { ...process.env, RELEASE_TAG: 'v0.1.1-rc.1', GITHUB_REPOSITORY: 'YOUR_OWNER/YOUR_REPO' },
  }) };
}

test('Tauri 2 原生制品生成全部平台并保留预发布版本和 URL 编码', t => {
  const f = fixture(t);
  const r = f.run();
  assert.equal(r.status, 0, r.stderr);
  const manifest = JSON.parse(readFileSync(join(f.cwd, 'latest.json'), 'utf8'));
  assert.equal(manifest.version, '0.1.1-rc.1');
  assert.equal(Object.keys(manifest.platforms).length, 9);
  assert.match(manifest.platforms['darwin-aarch64'].url, /Agent%20Manager_aarch64.app.tar.gz$/);
  assert.match(manifest.platforms['linux-x86_64'].url, /\.AppImage$/);
  assert.match(manifest.platforms['windows-x86_64-msi'].url, /\.msi$/);
  assert.match(manifest.platforms['windows-x86_64-nsis'].url, /-setup.exe$/);
  assert.equal(manifest.platforms['darwin-x86_64'].signature, 'test signature');
});
for (const failure of ['missing signature', 'empty signature', 'empty bundle', 'duplicate bundle', 'legacy format']) {
  test(`拒绝 ${failure}，不输出残缺更新清单`, t => {
    const f = fixture(t);
    const name = bundles[2];
    if (failure === 'missing signature') rmSync(join(f.dir, `${name}.sig`));
    if (failure === 'empty signature') writeFileSync(join(f.dir, `${name}.sig`), ' \n');
    if (failure === 'empty bundle') writeFileSync(join(f.dir, name), '');
    if (failure === 'duplicate bundle') writeFileSync(join(f.dir, 'other.AppImage'), 'duplicate');
    if (failure === 'legacy format') {
      rmSync(join(f.dir, name));
      writeFileSync(join(f.dir, `${name}.tar.gz`), 'legacy bundle');
    }
    assert.notEqual(f.run().status, 0);
    assert.throws(() => readFileSync(join(f.cwd, 'latest.json')), { code: 'ENOENT' });
  });
}
