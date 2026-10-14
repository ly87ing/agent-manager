import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

const script = fileURLToPath(new URL('./configure-release.mjs', import.meta.url));
function fixture(t) {
  const cwd = mkdtempSync(join(tmpdir(), 'release-config-'));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  mkdirSync(join(cwd, 'src-tauri'));
  const path = join(cwd, 'src-tauri', 'tauri.conf.json');
  const original = JSON.stringify({ version: '0.1.0', identifier: 'YOUR_APP_ID', plugins: { other: { enabled: true } } });
  writeFileSync(path, original);
  return { path, original, run: (env = {}) => spawnSync(process.execPath, [script], {
    cwd, encoding: 'utf8', env: {
      ...process.env, RELEASE_TAG: 'v0.1.0-rc.3', TAURI_UPDATER_PUBKEY: 'TEST_PUBLIC_KEY',
      APPLE_SIGNING_ENABLED: 'false', WINDOWS_SIGNING_ENABLED: 'false', ...env,
    },
  }) };
}

for (const version of ['0.1.0-alpha.1', '0.1.0-beta.2', '0.1.0-rc.3', '0.1.0']) {
  test(`runtime configuration retains full release version ${version}`, t => {
    const f = fixture(t);
    const result = f.run({ RELEASE_TAG: `v${version}` });
    assert.equal(result.status, 0, result.stderr);
    const config = JSON.parse(readFileSync(f.path, 'utf8'));
    assert.equal(config.version, version);
    assert.equal(config.identifier, 'YOUR_APP_ID');
    assert.deepEqual(config.plugins.other, { enabled: true });
    assert.equal(config.plugins.updater.pubkey, 'TEST_PUBLIC_KEY');
    assert.equal(config.bundle.createUpdaterArtifacts, true);
    if (process.platform === 'darwin') assert.equal(config.bundle.macOS.signingIdentity, '-');
    const configured = readFileSync(f.path, 'utf8');
    assert.equal(f.run({ RELEASE_TAG: `v${version}` }).status, 0);
    assert.equal(readFileSync(f.path, 'utf8'), configured);
  });
}

for (const env of [
  { RELEASE_TAG: '' }, { RELEASE_TAG: 'v0.1.0-unrecognized.1' }, { RELEASE_TAG: 'v0.2.0-rc.3' },
  { TAURI_UPDATER_PUBKEY: '' }, { TAURI_UPDATER_PUBKEY: 'YOUR_UPDATER_PUBKEY' },
]) {
  test(`invalid release configuration leaves source bytes unchanged: ${JSON.stringify(env)}`, t => {
    const f = fixture(t);
    assert.notEqual(f.run(env).status, 0);
    assert.equal(readFileSync(f.path, 'utf8'), f.original);
  });
}
