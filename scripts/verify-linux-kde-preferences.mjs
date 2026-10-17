// Actual KConfig/CLI reads inside the disposable KDE settings fixture.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync,
  readlinkSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';

export function verifyPreferences(engine, inherited, settingsRoot) {
  assert.equal(process.platform, 'linux');
  assert.equal(inherited.CI, 'true');
  assert.equal(inherited.RUNNER_ENVIRONMENT, 'github-hosted');
  assert.equal(inherited.AM_ISOLATED_LINUX_SETTINGS, '1');
  assert(process.getuid() !== 0);
  assert.equal(dirname(realpathSync(settingsRoot)), realpathSync(inherited.RUNNER_TEMP));
  assert(basename(settingsRoot).startsWith('kde-settings.'));
  assert.equal(lstatSync(settingsRoot).uid, process.getuid());
  const root = mkdtempSync(join(settingsRoot, 'preferences.'));
  const config = join(root, 'config');
  const defaults = join(root, 'defaults');
  const local = join(config, 'plasma-localerc');
  const global = join(defaults, 'kdeglobals');
  const env = { ...inherited, HOME: join(root, 'missing-home'),
    XDG_CONFIG_HOME: config, XDG_CONFIG_DIRS: defaults,
    XDG_CACHE_HOME: join(root, 'missing-cache'), XDG_DATA_HOME: join(root, 'missing-data'),
    XDG_STATE_HOME: join(root, 'missing-state'), AGENT_MANAGER_DATA_DIR: join(root, 'missing-manager'),
    XDG_CURRENT_DESKTOP: 'KDE', KDE_SKIP_KDERC: '1', LANG: 'de_DE.UTF-8',
    LC_ALL: 'C.UTF-8', LANGUAGE: 'it_IT' };
  const tree = path => {
    const values = [];
    for (const name of readdirSync(path).sort()) {
      const child = join(path, name);
      const stat = lstatSync(child);
      values.push([child, stat.mode, stat.uid, stat.gid, stat.mtimeMs,
        stat.isSymbolicLink() ? readlinkSync(child) : stat.isFile() ? readFileSync(child).toString('base64') : null]);
      if (stat.isDirectory()) values.push(...tree(child));
    }
    return values;
  };
  const check = (expected, spec) => {
    const before = tree(root);
    const result = spawnSync(engine, ['environment:check', ...(spec ? ['--spec', JSON.stringify(spec)] : []), '--json'],
      { env, encoding: 'utf8', timeout: 30_000, maxBuffer: 1024 * 1024 });
    assert.equal(result.status, 0, result.stderr || result.error || result.stdout);
    const response = JSON.parse(result.stdout);
    assert.equal(response.ok, true, JSON.stringify(response));
    const rows = Object.fromEntries(response.data.checks.map(row => [row.id, row]));
    for (const [id, value] of Object.entries(expected)) {
      assert.equal(rows[id].observed, value, JSON.stringify(rows[id]));
      assert(rows[id].source.startsWith('KDE plasma-localerc'), JSON.stringify(rows[id]));
    }
    assert.deepEqual(tree(root), before, 'Inspection mutated the isolated configuration tree');
    for (const key of ['HOME', 'XDG_CACHE_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME', 'AGENT_MANAGER_DATA_DIR']) {
      assert.equal(existsSync(env[key]), false, `Inspection created ${key}`);
    }
    return rows;
  };
  try {
    mkdirSync(config);
    mkdirSync(defaults);
    writeFileSync(local, '[Formats]\nLANG=en_GB.UTF-8\nLC_TIME=fr_FR.UTF-8\n\n[Translations]\nLANGUAGE=zh_TW:en_GB\n\n[Untargeted]\nKeep=sentinel\n');
    writeFileSync(global, '[Formats]\nLANG=fr_FR.UTF-8\n[Translations]\nLANGUAGE=fr_FR\n');
    const rows = check({ 'language.preferred': 'zh-TW, en-GB', 'language.region': 'GB',
      'language.locale': 'en_GB.UTF-8', 'language.formats': '{"LC_TIME":"fr_FR.UTF-8"}' },
    { languages: ['zh-TW', 'en-GB'], region: 'GB' });
    assert.equal(rows['language.preferred'].state, 'match');
    assert.equal(rows['language.region'].state, 'match');

    // A --file/NoGlobals query would incorrectly return the local values here.
    writeFileSync(global, '[Formats]\nLANG[$i]=de_DE.UTF-8\n[Translations]\nLANGUAGE[$i]=de_DE:en_GB\n');
    check({ 'language.preferred': 'de-DE, en-GB', 'language.region': 'DE', 'language.locale': 'de_DE.UTF-8' });

    writeFileSync(global, '[Formats]\nLANG=fr_FR.UTF-8\n[Translations]\nLANGUAGE=fr_FR:en_GB\n');
    rmSync(local);
    rmSync(config, { recursive: true });
    check({ 'language.preferred': 'fr-FR, en-GB', 'language.region': 'FR', 'language.locale': 'fr_FR.UTF-8' });
    assert.equal(existsSync(config), false, 'Inherited read created the user configuration directory');

    rmSync(global);
    check({ 'language.preferred': null, 'language.region': null, 'language.locale': null,
      'language.formats': '{}' });
    assert.equal(existsSync(config), false);

    mkdirSync(config);
    writeFileSync(local, '[Formats]\nLANG=sr_RS@latin\nLC_TIME=en_GB.UTF-8\n[Translations]\nLANGUAGE=sr_RS@latin\n');
    check({ 'language.preferred': null, 'language.region': 'RS', 'language.locale': 'sr_RS@latin',
      'language.formats': '{"LC_TIME":"en_GB.UTF-8"}' });
    writeFileSync(local, '[Formats]\nLANG=en_GB.UTF-8\n[Translations]\nLANGUAGE=en_US:\n');
    check({ 'language.preferred': null, 'language.region': 'GB' });
    console.log('KDE native KConfig reads: saved order, independent formats, global immutable/default inheritance, missing sources and zero writes verified');
  } finally {
    rmSync(root, { recursive: true });
  }
}
