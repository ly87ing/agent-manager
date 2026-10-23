import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, renameSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { createHash } from 'node:crypto';

const script = fileURLToPath(new URL('./assemble-updater.mjs', import.meta.url));
const bundles = [
  'Agent Manager_aarch64.app.tar.gz', 'Agent Manager_x64.app.tar.gz',
  'Agent-Manager_amd64.AppImage', 'Agent-Manager_0.1.1_x64_en-US.msi', 'Agent-Manager_x64-setup.exe',
  'Agent-Manager_amd64.deb',
  'Agent-Manager_0.1.1_aarch64.AppImage', 'Agent-Manager_0.1.1_arm64.deb', 'Agent-Manager_0.1.1_arm64-setup.exe',
];
function fixture(t) {
  const cwd = mkdtempSync(join(tmpdir(), 'updater-manifest-'));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  const dir = join(cwd, 'updater-assets');
  mkdirSync(dir);
  for (const name of bundles) {
    writeFileSync(join(dir, name), 'test bundle');
    writeFileSync(join(dir, `${name}.sig`), `signature for ${name}\n`);
  }
  return { cwd, dir, run: () => spawnSync(process.execPath, [script], {
    cwd, encoding: 'utf8', env: { ...process.env, RELEASE_TAG: 'v0.1.1-rc.1', GITHUB_REPOSITORY: 'YOUR_OWNER/YOUR_REPO' },
  }) };
}

test('Tauri 2 原生制品生成全部平台并保留预发布版本和 URL 编码', t => {
  const f = fixture(t);
  mkdirSync(join(f.cwd, 'src-tauri'));
  const configPath = join(f.cwd, 'src-tauri', 'tauri.conf.json');
  writeFileSync(configPath, JSON.stringify({ version: '0.1.1' }));
  const configured = spawnSync(process.execPath, [fileURLToPath(new URL('./configure-release.mjs', import.meta.url))], {
    cwd: f.cwd, encoding: 'utf8', env: { ...process.env, RELEASE_TAG: 'v0.1.1-rc.1', TAURI_UPDATER_PUBKEY: 'TEST_PUBLIC_KEY', WINDOWS_SIGNING_ENABLED: 'false', APPLE_SIGNING_ENABLED: 'false' },
  });
  assert.equal(configured.status, 0, configured.stderr);
  const r = f.run();
  assert.equal(r.status, 0, r.stderr);
  const manifest = JSON.parse(readFileSync(join(f.cwd, 'latest.json'), 'utf8'));
  assert.equal(manifest.version, '0.1.1-rc.1');
  assert.equal(manifest.version, JSON.parse(readFileSync(configPath, 'utf8')).version);
  assert.equal(Object.keys(manifest.platforms).length, 15);
  assert.match(manifest.platforms['darwin-aarch64'].url, /Agent%20Manager_aarch64.app.tar.gz$/);
  assert.match(manifest.platforms['linux-x86_64'].url, /\.AppImage$/);
  assert.match(manifest.platforms['linux-x86_64-appimage'].url, /\.AppImage$/);
  assert.match(manifest.platforms['linux-x86_64-deb'].url, /\.deb$/);
  assert.equal(manifest.platforms['linux-x86_64-deb'].signature, `signature for ${bundles[5]}`);
  assert.equal(manifest.platforms['linux-x86_64-appimage'].signature, `signature for ${bundles[2]}`);
  assert.match(manifest.platforms['windows-x86_64-msi'].url, /\.msi$/);
  assert.match(manifest.platforms['windows-x86_64-nsis'].url, /-setup.exe$/);
  assert.equal(manifest.platforms['windows-x86_64'].signature, `signature for ${bundles[3]}`);
  assert.equal(manifest.platforms['windows-aarch64'].signature, `signature for ${bundles[8]}`);
  assert.equal(manifest.platforms['windows-aarch64-nsis'].signature, `signature for ${bundles[8]}`);
  assert.equal(manifest.platforms['windows-aarch64-msi'], undefined);
  assert.equal(manifest.platforms['linux-aarch64'].signature, `signature for ${bundles[6]}`);
  assert.equal(manifest.platforms['linux-aarch64-appimage'].signature, `signature for ${bundles[6]}`);
  assert.equal(manifest.platforms['linux-aarch64-deb'].signature, `signature for ${bundles[7]}`);
  assert.match(manifest.platforms['linux-aarch64-appimage'].url, /_aarch64\.AppImage$/);
  assert.match(manifest.platforms['linux-aarch64-deb'].url, /_arm64\.deb$/);
  assert.match(manifest.platforms['windows-aarch64-nsis'].url, /_arm64-setup\.exe$/);
  assert.equal(manifest.platforms['darwin-x86_64'].signature, `signature for ${bundles[1]}`);
  const sums = readFileSync(join(f.cwd, 'SHA256SUMS.txt'), 'utf8').trim().split('\n');
  assert.equal(sums.length, bundles.length * 2 + 1);
  for (const line of sums) {
    const [hash, name] = [line.slice(0, 64), line.slice(66)];
    const file = name === 'latest.json' ? join(f.cwd, name) : join(f.dir, name);
    assert.equal(hash, createHash('sha256').update(readFileSync(file)).digest('hex'));
  }
});

for (const name of bundles.slice(6)) {
  for (const failure of ['missing bundle', 'missing signature', 'empty signature', 'empty bundle', 'duplicate bundle']) {
    test(`ARM64 ${name}: ${failure} cannot fall back to x64`, t => {
      const f = fixture(t);
      if (failure === 'missing bundle') rmSync(join(f.dir, name));
      if (failure === 'missing signature') rmSync(join(f.dir, `${name}.sig`));
      if (failure === 'empty signature') writeFileSync(join(f.dir, `${name}.sig`), ' \n');
      if (failure === 'empty bundle') writeFileSync(join(f.dir, name), '');
      if (failure === 'duplicate bundle') writeFileSync(join(f.dir, `duplicate_${name}`), 'duplicate');
      assert.notEqual(f.run().status, 0);
      assert.throws(() => readFileSync(join(f.cwd, 'latest.json')), { code: 'ENOENT' });
      assert.throws(() => readFileSync(join(f.cwd, 'SHA256SUMS.txt')), { code: 'ENOENT' });
    });
  }
}

for (const name of ['unknown.AppImage', 'Agent_riscv64.deb', 'Agent_x64_arm64.deb', 'Agent_aarch64_x64-setup.exe', 'Agent_arm64_en-US.msi']) {
  test(`rejects unsupported or ambiguous updater bundle ${name}`, t => {
    const f = fixture(t);
    writeFileSync(join(f.dir, name), 'unclassified bundle');
    writeFileSync(join(f.dir, `${name}.sig`), 'signature');
    assert.notEqual(f.run().status, 0);
    assert.throws(() => readFileSync(join(f.cwd, 'latest.json')), { code: 'ENOENT' });
    assert.throws(() => readFileSync(join(f.cwd, 'SHA256SUMS.txt')), { code: 'ENOENT' });
  });
}

test('keeps the legacy x64 MSI filename without a language suffix', t => {
  const f = fixture(t);
  const renamed = 'Agent-Manager_x64.msi';
  for (const suffix of ['', '.sig']) renameSync(join(f.dir, bundles[3] + suffix), join(f.dir, renamed + suffix));
  const r = f.run();
  assert.equal(r.status, 0, r.stderr);
  const manifest = JSON.parse(readFileSync(join(f.cwd, 'latest.json'), 'utf8'));
  assert.match(manifest.platforms['windows-x86_64-msi'].url, /_x64\.msi$/);
  assert.equal(manifest.platforms['windows-x86_64'].signature, `signature for ${bundles[3]}`);
});

for (const failure of ['missing bundle', 'missing signature', 'empty signature', 'empty bundle', 'duplicate bundle']) {
  test(`rejects a deb ${failure} before publishing a manifest`, t => {
    const f = fixture(t);
    const name = bundles[5];
    if (failure === 'missing bundle') rmSync(join(f.dir, name));
    if (failure === 'missing signature') rmSync(join(f.dir, `${name}.sig`));
    if (failure === 'empty signature') writeFileSync(join(f.dir, `${name}.sig`), ' \n');
    if (failure === 'empty bundle') writeFileSync(join(f.dir, name), '');
    if (failure === 'duplicate bundle') writeFileSync(join(f.dir, 'other.deb'), 'duplicate');
    assert.notEqual(f.run().status, 0);
    assert.throws(() => readFileSync(join(f.cwd, 'latest.json')), { code: 'ENOENT' });
    assert.throws(() => readFileSync(join(f.cwd, 'SHA256SUMS.txt')), { code: 'ENOENT' });
  });
}
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
    assert.throws(() => readFileSync(join(f.cwd, 'SHA256SUMS.txt')), { code: 'ENOENT' });
  });
}

test('重跑草稿时替换旧清单，不把校验文件自身或旧 latest.json 加入摘要', t => {
  const f = fixture(t);
  writeFileSync(join(f.dir, 'latest.json'), '{"version":"old"}');
  writeFileSync(join(f.dir, 'SHA256SUMS.txt'), 'old checksum list');
  const r = f.run();
  assert.equal(r.status, 0, r.stderr);
  const sums = readFileSync(join(f.cwd, 'SHA256SUMS.txt'), 'utf8').trim().split('\n');
  assert.equal(sums.length, bundles.length * 2 + 1);
  assert.equal(sums.filter(line => line.endsWith('  latest.json')).length, 1);
  assert.equal(sums.filter(line => line.endsWith('  SHA256SUMS.txt')).length, 0);
  const digest = createHash('sha256').update(readFileSync(join(f.cwd, 'latest.json'))).digest('hex');
  assert(sums.includes(`${digest}  latest.json`));
});
