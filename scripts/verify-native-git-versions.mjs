// Real uv/Git, a private local repository and an explicitly selected existing interpreter.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, isAbsolute, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

assert.equal(process.env.AM_ISOLATED_NATIVE_UV, '1');
for (const key of ['AM_NATIVE_UV', 'AM_NATIVE_PYTHON']) {
  assert(process.env[key] && isAbsolute(process.env[key]), `Select an existing absolute ${key}`);
  assert(lstatSync(realpathSync(process.env[key])).isFile());
}
const windows = process.platform === 'win32';
const python = realpathSync(process.env.AM_NATIVE_PYTHON);
const pythonHash = createHash('sha256').update(readFileSync(python)).digest('hex');
const root = mkdtempSync(join(tmpdir(), 'am git-native-'));
const home = join(root, 'home'), data = join(root, 'data'), bin = join(home, 'bin');
const tools = join(root, 'tools'), repo = join(root, 'repository');
for (const path of [home, bin, tools, repo]) mkdirSync(path, { recursive: true });
const gitConfig = join(root, 'empty-gitconfig'); writeFileSync(gitConfig, '');
const uv = join(tools, windows ? 'uv.exe' : 'uv');
copyFileSync(realpathSync(process.env.AM_NATIVE_UV), uv);
if (!windows) chmodSync(uv, 0o755);
const engine = join(process.env.CARGO_TARGET_DIR ? resolve(process.env.CARGO_TARGET_DIR) : resolve('engine-rs/target'), 'debug', windows ? 'agent-manager-engine.exe' : 'agent-manager-engine');
const inherited = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(UV_|PIP_|PYTHON|CONDA|VIRTUAL_ENV|XDG_|GIT_|APPDATA$|LOCALAPPDATA$|HOME$|USERPROFILE$|AGENT_MANAGER_|https?_proxy$|all_proxy$|no_proxy$)/i.test(key)));
if (windows) delete inherited.Path;
const env = { ...inherited, HOME: home, USERPROFILE: home,
  PATH: [tools, bin, process.env.PATH ?? process.env.Path ?? ''].join(delimiter),
  XDG_CONFIG_HOME: join(home, '.config'), XDG_DATA_HOME: join(home, '.local/share'), XDG_CACHE_HOME: join(home, '.cache'),
  APPDATA: join(home, 'AppData/Roaming'), LOCALAPPDATA: join(home, 'AppData/Local'),
  AGENT_MANAGER_DATA_DIR: data, AGENT_MANAGER_USER_PATH: [bin, tools].join(delimiter),
  UV_NO_CONFIG: '1', UV_NO_INDEX: '1', UV_CACHE_DIR: join(root, 'cache'), UV_PYTHON: python,
  UV_PYTHON_DOWNLOADS: 'never', UV_PYTHON_INSTALL_DIR: join(root, 'python'),
  UV_PYTHON_INSTALL_BIN: '0', UV_PYTHON_INSTALL_REGISTRY: '0', UV_PYTHON_NO_REGISTRY: '1',
  GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: gitConfig, GIT_TERMINAL_PROMPT: '0',
  GIT_AUTHOR_NAME: 'Fixture', GIT_AUTHOR_EMAIL: 'fixture@example.invalid',
  GIT_COMMITTER_NAME: 'Fixture', GIT_COMMITTER_EMAIL: 'fixture@example.invalid',
  GIT_CONFIG_COUNT: '1', GIT_CONFIG_KEY_0: `url.${pathToFileURL(repo).href}.insteadOf`,
  GIT_CONFIG_VALUE_0: 'https://example.invalid/owner/am-git-fixture',
  HTTP_PROXY: 'http://127.0.0.1:9', HTTPS_PROXY: 'http://127.0.0.1:9',
  http_proxy: 'http://127.0.0.1:9', https_proxy: 'http://127.0.0.1:9', PYTHONDONTWRITEBYTECODE: '1' };
const id = 'git:example.invalid/owner/am-git-fixture';
const program = join(data, 'programs/git/example.invalid~owner~am-git-fixture');
const meta = '.agent-manager-program.json';

function run(binary, args, expectedStatus = 0) {
  const result = spawnSync(binary, args, { cwd: repo, env, encoding: 'utf8', timeout: 45_000, maxBuffer: 1_048_576 });
  assert.ifError(result.error);
  assert.equal(result.status, expectedStatus, `${binary}: ${result.stdout}\n${result.stderr}`);
  return result.stdout.trim();
}
function cli(command, args, ok = true) {
  const result = JSON.parse(run(engine, [command, ...args, '--json'], ok ? 0 : 1));
  assert.equal(result.ok, ok, JSON.stringify(result));
  return result;
}
function action(command, action, commit, ok = true) {
  return cli(command, ['--program', id, '--action', action, '--version', commit, ...(action === 'install' ? ['--entry-dir', bin] : [])], ok);
}
function origin(directory) {
  const files = [];
  function walk(path) {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const child = join(path, entry.name);
      if (entry.isDirectory()) walk(child);
      else if (entry.name === 'direct_url.json') files.push(child);
    }
  }
  walk(join(directory, 'tools'));
  assert.equal(files.length, 1);
  return files[0];
}
function check(commit, version) {
  const directory = join(program, commit);
  const recorded = JSON.parse(readFileSync(origin(directory), 'utf8'));
  assert.equal(recorded.url, 'https://example.invalid/owner/am-git-fixture');
  assert.equal(recorded.vcs_info.commit_id, commit);
  assert.equal(JSON.parse(readFileSync(join(directory, meta), 'utf8')).version, commit);
  assert.equal(realpathSync(join(program, 'current')), realpathSync(directory));
  assert.equal(run(join(directory, 'bin', windows ? 'am-git-fixture.exe' : 'am-git-fixture'), []), version);
}
function commit(version) {
  writeFileSync(join(repo, 'pyproject.toml'), `[build-system]\nrequires=[]\nbuild-backend="backend"\nbackend-path=["."]\n[project]\nname="am-git-fixture"\nversion="${version}"\n`);
  writeFileSync(join(repo, 'version.txt'), version);
  run('git', ['add', '.']);
  run('git', ['commit', '-qm', `test: add isolated Git tool fixture version ${version}`]);
  const commit = run('git', ['rev-parse', 'HEAD']);
  assert.match(commit, /^[0-9a-f]{40}$/);
  return commit;
}

try {
  // This standard-library-only PEP 517 fixture creates its own tiny offline wheel.
  writeFileSync(join(repo, 'backend.py'), `import os, pathlib, zipfile
def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    version = pathlib.Path(__file__).with_name('version.txt').read_text().strip()
    name = 'am_git_fixture-' + version + '-py3-none-any.whl'
    prefix = 'am_git_fixture-' + version + '.dist-info/'
    files = {'am_git_fixture.py': 'def main():\\n    print(' + repr(version) + ')\\n',
        prefix+'METADATA': 'Metadata-Version: 2.4\\nName: am-git-fixture\\nVersion: '+version+'\\n\\n',
        prefix+'WHEEL': 'Wheel-Version: 1.0\\nRoot-Is-Purelib: true\\nTag: py3-none-any\\n',
        prefix+'entry_points.txt': '[console_scripts]\\nam-git-fixture = am_git_fixture:main\\n'}
    files[prefix+'RECORD'] = ''.join(key+',,\\n' for key in files)+prefix+'RECORD,,\\n'
    with zipfile.ZipFile(os.path.join(wheel_directory,name),'w') as wheel:
        for key,value in files.items(): wheel.writestr(key,value)
    return name
`);
  run('git', ['init', '-q', '--initial-branch=main']);
  const first = commit('1.0.0'), second = commit('2.0.0');
  const skill = join(home, '.claude/skills/am-git-fixture'); mkdirSync(skill, { recursive: true });
  writeFileSync(join(skill, 'SKILL.md'), '---\nname: am-git-fixture\ndescription: private fixture\nmetadata:\n  requires:\n    bins: [am-git-fixture]\n---\nFixture\n');
  cli('programs:declare', ['--bin', 'am-git-fixture', '--program', id]);
  action('programs:preview', 'install', first);
  assert(!existsSync(program), 'Preview created the candidate');
  action('programs:apply', 'install', first); check(first, '1.0.0');
  action('programs:apply', 'upgrade', second); check(second, '2.0.0');
  action('programs:apply', 'switch', first); check(first, '1.0.0');
  const file = origin(join(program, second)), original = readFileSync(file);
  const changed = JSON.parse(original); changed.vcs_info.commit_id = first;
  writeFileSync(file, JSON.stringify(changed));
  for (const operation of ['switch', 'upgrade']) {
    const refused = action('programs:apply', operation, second, false);
    assert.equal(refused.errors[0].code, 'PROGRAM_VERSION_UNVERIFIED');
    assert.equal(realpathSync(join(program, 'current')), realpathSync(join(program, first)));
    assert.deepEqual(readFileSync(file), Buffer.from(JSON.stringify(changed)));
  }
  writeFileSync(file, original);
  action('programs:apply', 'upgrade', second); check(second, '2.0.0');
  for (const operation of ['unlink', 'link', 'unlink']) action('programs:apply', operation, second);
  writeFileSync(file, JSON.stringify(changed));
  const refusedLink = action('programs:apply', 'link', second, false);
  assert.equal(refusedLink.errors[0].code, 'PROGRAM_VERSION_UNVERIFIED');
  assert.equal(realpathSync(join(program, 'current')), realpathSync(join(program, second)));
  assert(!existsSync(join(bin, windows ? 'am-git-fixture.cmd' : 'am-git-fixture')));
  assert.deepEqual(readFileSync(file), Buffer.from(JSON.stringify(changed)));
  writeFileSync(file, original);
  action('programs:apply', 'link', second); check(second, '2.0.0');
  assert.equal(createHash('sha256').update(readFileSync(python)).digest('hex'), pythonHash);
  assert(!existsSync(env.UV_PYTHON_INSTALL_DIR) || readdirSync(env.UV_PYTHON_INSTALL_DIR).length === 0);
  console.log('Real Git/uv: offline commit installation, actual origin/entry execution, upgrade/switch/reuse, linking and stale commit refusal passed; existing interpreter bytes preserved.');
  rmSync(root, { recursive: true });
} catch (error) {
  console.error(`Retained private Git fixture: ${root}`);
  throw error;
}
