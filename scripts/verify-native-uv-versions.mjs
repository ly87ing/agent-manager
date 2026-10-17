// Real uv plus an explicitly selected existing interpreter and private offline wheels.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, copyFileSync, chmodSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, isAbsolute, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

assert.equal(process.env.AM_ISOLATED_NATIVE_UV, '1');
assert(process.argv.slice(2).every(arg => ['--custom-root', '--managed'].includes(arg)));
const custom = process.argv.includes('--custom-root');
const managed = process.argv.includes('--managed');
assert(!(managed && custom));
for (const name of ['AM_NATIVE_UV', 'AM_NATIVE_PYTHON']) {
  assert(process.env[name] && isAbsolute(process.env[name]), `Select an existing absolute ${name}`);
  assert(lstatSync(realpathSync(process.env[name])).isFile(), `${name} must name an existing ordinary executable`);
}
const windows = process.platform === 'win32';
const originalPython = realpathSync(process.env.AM_NATIVE_PYTHON);
const pythonBefore = readFileSync(originalPython);
const pythonIdentity = lstatSync(originalPython);
const root = mkdtempSync(join(tmpdir(), 'am uv-version-native-'));
const home = join(root, 'home'), data = join(root, 'data'), bin = join(home, 'bin');
const tools = join(root, 'tools'), wheels = join(root, 'wheels'), uvTools = join(home, custom ? 'custom-extension-tools' : 'uv/data/tools');
for (const path of [home, bin, tools, wheels]) mkdirSync(path, { recursive: true });
const uv = join(tools, windows ? 'uv.exe' : 'uv');
copyFileSync(realpathSync(process.env.AM_NATIVE_UV), uv);
if (!windows) chmodSync(uv, 0o755);
const engine = resolve(`engine-rs/target/debug/agent-manager-engine${windows ? '.exe' : ''}`);
const inherited = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(UV_|PIP_|PYTHON|CONDA|VIRTUAL_ENV|XDG_|APPDATA$|LOCALAPPDATA$|HOME$|USERPROFILE$|AGENT_MANAGER_|https?_proxy$|all_proxy$)/i.test(key)));
if (windows) delete inherited.Path;
const env = { ...inherited, HOME: home, USERPROFILE: home,
  XDG_CONFIG_HOME: join(home, '.config'), XDG_DATA_HOME: join(home, '.local/share'), XDG_CACHE_HOME: join(home, '.cache'),
  APPDATA: join(home, 'AppData/Roaming'), LOCALAPPDATA: join(home, 'AppData/Local'),
  PATH: [tools, bin, process.env.PATH ?? process.env.Path ?? ''].join(delimiter),
  AGENT_MANAGER_DATA_DIR: data, AGENT_MANAGER_USER_PATH: [bin, tools].join(delimiter),
  UV_NO_CONFIG: '1', UV_NO_INDEX: '1', UV_FIND_LINKS: pathToFileURL(wheels).href,
  UV_TOOL_DIR: uvTools, UV_TOOL_BIN_DIR: bin, UV_CACHE_DIR: join(root, 'cache'),
  UV_PYTHON: originalPython, UV_PYTHON_DOWNLOADS: 'automatic', UV_LINK_MODE: 'hardlink',
  UV_PYTHON_INSTALL_DIR: join(root, 'python'), UV_PYTHON_CACHE_DIR: join(root, 'python-archives'),
  UV_PYTHON_BIN_DIR: join(root, 'python-bin'), UV_PYTHON_INSTALL_BIN: '0',
  UV_PYTHON_INSTALL_REGISTRY: '0', UV_PYTHON_NO_REGISTRY: '1', PYTHONDONTWRITEBYTECODE: '1' };

function run(program, args, windowsVerbatimArguments = false) {
  const result = spawnSync(program, args, { env, cwd: root, encoding: 'utf8', timeout: 45_000, maxBuffer: 1_048_576, windowsVerbatimArguments });
  assert.ifError(result.error);
  assert.equal(result.status, 0, `${program}: ${result.stdout}\n${result.stderr}`);
  return result.stdout;
}
function cli(command, args = []) {
  const result = JSON.parse(run(engine, [command, ...args, '--json']));
  assert.equal(result.ok, true, JSON.stringify(result));
  return result.data;
}

function crc32(bytes) {
  let value = 0xffffffff;
  for (const byte of bytes) {
    value ^= byte;
    for (let bit = 0; bit < 8; bit++) value = (value >>> 1) ^ (0xedb88320 & -(value & 1));
  }
  return (value ^ 0xffffffff) >>> 0;
}
function wheel(version) {
  const stem = `am_uv_version_fixture-${version}.dist-info`;
  const files = new Map([
    ['am_uv_version_fixture.py', `def main():\n    print(${JSON.stringify(version)})\n`],
    [`${stem}/METADATA`, `Metadata-Version: 2.4\nName: am-uv-version-fixture\nVersion: ${version}\nRequires-Python: >=3.10\n\nPrivate offline fixture\n`],
    [`${stem}/WHEEL`, 'Wheel-Version: 1.0\nGenerator: native-version-fixture\nRoot-Is-Purelib: true\nTag: py3-none-any\n'],
    [`${stem}/entry_points.txt`, '[console_scripts]\nam-uv-version-fixture = am_uv_version_fixture:main\n'],
  ]);
  const records = [...files].map(([name, contents]) => `${name},sha256=${createHash('sha256').update(contents).digest('base64url')},${Buffer.byteLength(contents)}`);
  files.set(`${stem}/RECORD`, `${records.join('\n')}\n${stem}/RECORD,,\n`);
  const local = [], central = []; let offset = 0;
  for (const [name, text] of files) {
    const filename = Buffer.from(name), content = Buffer.from(text), crc = crc32(content);
    const header = Buffer.alloc(30); header.writeUInt32LE(0x04034b50); header.writeUInt16LE(20, 4);
    header.writeUInt32LE(crc, 14); header.writeUInt32LE(content.length, 18); header.writeUInt32LE(content.length, 22); header.writeUInt16LE(filename.length, 26);
    local.push(header, filename, content);
    const directory = Buffer.alloc(46); directory.writeUInt32LE(0x02014b50); directory.writeUInt16LE(20, 4); directory.writeUInt16LE(20, 6);
    directory.writeUInt32LE(crc, 16); directory.writeUInt32LE(content.length, 20); directory.writeUInt32LE(content.length, 24); directory.writeUInt16LE(filename.length, 28); directory.writeUInt32LE(offset, 42);
    central.push(directory, filename); offset += header.length + filename.length + content.length;
  }
  const centralBytes = Buffer.concat(central), end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50); end.writeUInt16LE(files.size, 8); end.writeUInt16LE(files.size, 10);
  end.writeUInt32LE(centralBytes.length, 12); end.writeUInt32LE(offset, 16);
  writeFileSync(join(wheels, `am_uv_version_fixture-${version}-py3-none-any.whl`), Buffer.concat([...local, centralBytes, end]));
}

let complete = false;
try {
  for (const version of ['1.0.0', '2.0.0']) wheel(version);
  const version = run(uv, ['--version']).trim();
  if (!managed) run(uv, ['--no-python-downloads', 'tool', 'install', '--force', 'am-uv-version-fixture==1.0.0']);
  const skill = join(home, '.claude/skills/uv-version'); mkdirSync(skill, { recursive: true });
  const declaration = managed
    ? '  openclaw:\n    requires:\n      bins: [am-uv-version-fixture]\n    install:\n      - kind: uv\n        package: am-uv-version-fixture\n        bins: [am-uv-version-fixture]\n'
    : '  requires:\n    bins: [am-uv-version-fixture]\n';
  writeFileSync(join(skill, 'SKILL.md'), `---\nname: uv-version\ndescription: private offline fixture\nmetadata:\n${declaration}---\nPrivate fixture\n`);
  const entry = join(bin, windows ? `am-uv-version-fixture.${managed ? 'cmd' : 'exe'}` : 'am-uv-version-fixture');
  const id = 'pypi:am-uv-version-fixture';
  const element = () => cli('programs:status').elements.find(item => item.program === id);
  const output = () => {
    if (!managed || !windows) return run(entry, []).trim();
    assert(!/[\r\n%!"]/.test(entry), 'Private batch fixture path requires literal-safe characters');
    return run(process.env.ComSpec || 'cmd.exe', ['/d', '/v:off', '/s', '/c', `""${entry}""`], true).trim();
  };
  if (managed) {
    const args = ['--program', id, '--action', 'install', '--version', '1.0.0', '--entry-dir', bin];
    cli('programs:preview', args); assert(!existsSync(data));
    cli('programs:apply', args);
    assert.equal(output(), '1.0.0');
  } else {
    assert.equal(element().owner, 'uv-tool'); assert.equal(element().version, '1.0.0'); assert(!existsSync(data));
  }
  wheel('1.0+native');
  const actions = managed
    ? [['upgrade', 'v2.0.0', '2.0.0'], ['upgrade', '1.0', '1.0+native'], ['switch', 'v2.0.0', '2.0.0'], ['switch', '1.0', '1.0+native']]
    : [['upgrade', 'v2.0.0', '2.0.0'], ['switch', '1.0', '1.0+native']];
  for (const [action, target, expected] of actions) {
    const args = ['--program', id, '--action', action, '--version', target];
    const preview = cli('programs:preview', args);
    assert(preview.notes.join(' ').includes(managed ? 'completion and current selection' : 'any successful action'));
    cli('programs:apply', args);
    if (managed) {
      assert.equal(element().managed_current, target);
      const directory = join(data, 'programs/pypi/am-uv-version-fixture', target);
      assert.equal(realpathSync(join(directory, '..', 'current')), realpathSync(directory));
      assert(existsSync(join(directory, '.agent-manager-program.json')));
      const library = join(directory, 'tools/am-uv-version-fixture', windows ? 'Lib' : 'lib');
      const site = windows ? join(library, 'site-packages') : join(library, readdirSync(library).find(name => /^(python|pypy)\d/.test(name)), 'site-packages');
      const distribution = readdirSync(site).find(name => name === `am_uv_version_fixture-${expected}.dist-info`);
      assert(distribution);
      assert(readFileSync(join(site, distribution, 'METADATA'), 'utf8').includes(`\nVersion: ${expected}\n`));
    } else assert.equal(element().version, expected);
    assert.equal(output(), expected, 'The installed command did not use the new package');
  }
  const backups = cli('programs:status').backups.filter(item => item.program === id);
  assert.equal(backups.length, managed ? 0 : 2);
  assert.deepEqual(readFileSync(originalPython), pythonBefore);
  const pythonAfter = lstatSync(originalPython);
  assert.equal(pythonAfter.ino, pythonIdentity.ino); assert.equal(pythonAfter.mode, pythonIdentity.mode);
  for (const directory of [env.UV_PYTHON_INSTALL_DIR, env.UV_PYTHON_CACHE_DIR]) {
    assert(!existsSync(directory) || readdirSync(directory).length === 0, `Managed runtime provisioning appeared in ${directory}`);
  }
  console.log(`Native ${process.platform} ${version} (${managed ? 'managed' : custom ? 'custom' : 'standard'} root): offline real-package upgrade, entry execution, public/local PEP 440 matching and existing-interpreter preservation passed.`);
  complete = true;
} finally {
  if (complete) rmSync(root, { recursive: true });
  else console.error(`Retained private uv version fixture: ${root}`);
}
