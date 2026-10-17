// Real uv, private paths and a 404-only loopback mirror; no interpreter is installed.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync,
  readFileSync, readdirSync, readlinkSync, renameSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { delimiter, isAbsolute, join, resolve } from 'node:path';

assert.equal(process.env.AM_ISOLATED_NATIVE_UV, '1');
assert(['darwin', 'linux', 'win32'].includes(process.platform));
const uv = process.env.AM_NATIVE_UV;
assert(uv && isAbsolute(uv) && lstatSync(uv).isFile(), 'Supply the real uv binary explicitly');
const windows = process.platform === 'win32';
const engine = resolve(`engine-rs/target/debug/agent-manager-engine${windows ? '.exe' : ''}`);
const root = mkdtempSync(join(tmpdir(), 'am-uv-runtime-'));
const tools = join(root, 'tools');
mkdirSync(tools);
const copied = join(tools, windows ? 'uv.exe' : 'uv');
copyFileSync(uv, copied);
if (!windows) chmodSync(copied, 0o755);
const requests = [];
const server = createServer((request, response) => {
  requests.push(request.url);
  response.writeHead(404, { 'content-type': 'text/plain' });
  response.end('Private fixture: no interpreter archive exists.');
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const inherited = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(UV_|PIP_|PYTHON|CONDA|VIRTUAL_ENV|HTTP_PROXY|HTTPS_PROXY|ALL_PROXY)/i.test(key)));
function environment(name) {
  const base = join(root, name);
  const home = join(base, 'home');
  const bin = join(home, 'bin');
  const wheels = join(base, 'wheels');
  for (const path of [bin, wheels]) mkdirSync(path, { recursive: true });
  const env = { ...inherited, HOME: home, USERPROFILE: home,
    APPDATA: join(base, 'appdata'), LOCALAPPDATA: join(base, 'localappdata'),
    XDG_CONFIG_HOME: join(base, 'config'), XDG_CACHE_HOME: join(base, 'cache'),
    XDG_DATA_HOME: join(base, 'share'), XDG_STATE_HOME: join(base, 'state'),
    AGENT_MANAGER_DATA_DIR: join(base, 'manager'), AGENT_MANAGER_USER_PATH: [bin, tools].join(delimiter),
    PATH: [tools, bin, inherited.PATH ?? inherited.Path ?? ''].join(delimiter),
    UV_NO_CONFIG: '1', UV_NO_INDEX: '1', UV_FIND_LINKS: wheels,
    UV_CACHE_DIR: join(base, 'uv-cache'), UV_PYTHON: '3.13',
    UV_PYTHON_PREFERENCE: 'only-managed', UV_PYTHON_DOWNLOADS: 'automatic',
    UV_PYTHON_INSTALL_DIR: join(base, 'python'), UV_PYTHON_CACHE_DIR: join(base, 'python-archives'),
    UV_PYTHON_INSTALL_MIRROR: `http://127.0.0.1:${server.address().port}/python`,
    UV_PYTHON_BIN_DIR: join(base, 'python-bin'), UV_PYTHON_INSTALL_BIN: '0',
    UV_PYTHON_INSTALL_REGISTRY: '0', UV_PYTHON_NO_REGISTRY: '1', NO_PROXY: '127.0.0.1,localhost' };
  delete env.AGENT_MANAGER_LOCK_HELD;
  delete env.AGENT_MANAGER_LOCK_PID;
  if (windows) delete env.Path;
  return { base, home, bin, env };
}
function run(program, args, fixture) {
  return new Promise((resolve, reject) => {
    const child = spawn(program, args, { env: fixture.env, cwd: fixture.base,
      stdio: ['ignore', 'pipe', 'pipe'], timeout: 45_000 });
    let stdout = '', stderr = '';
    child.stdout.on('data', bytes => { stdout += bytes; assert(stdout.length < 1_048_576); });
    child.stderr.on('data', bytes => { stderr += bytes; assert(stderr.length < 1_048_576); });
    child.on('error', reject);
    child.on('close', status => resolve({ stdout, stderr, status }));
  });
}
function tree(path) {
  const entries = [];
  for (const name of readdirSync(path).sort()) {
    const child = join(path, name), stat = lstatSync(child);
    entries.push([child, stat.mode, stat.ino, stat.isSymbolicLink() ? readlinkSync(child)
      : stat.isFile() ? createHash('sha256').update(readFileSync(child)).digest('hex') : null]);
    if (stat.isDirectory()) entries.push(...tree(child));
  }
  return entries;
}
function noRuntime(fixture) {
  const files = key => existsSync(fixture.env[key]) ? readdirSync(fixture.env[key], { recursive: true }) : [];
  assert(files('UV_PYTHON_INSTALL_DIR').every(path => !/python(?:\d+(?:\.\d+)*)?(?:\.exe)?$/i.test(path)));
  assert(files('UV_PYTHON_CACHE_DIR').every(path => !/\.(gz|zip|zst|xz)$/i.test(path)));
}
let complete = false;
try {
  const control = environment('control');
  const version = await run(copied, ['--version'], control);
  assert.equal(version.status, 0, version.stderr);
  const attempted = await run(copied, ['tool', 'install', '--force', 'am-uv-fixture==1.0.0'], control);
  assert.notEqual(attempted.status, 0);
  assert(requests.length > 0, 'Control did not attempt a supported Python download');
  noRuntime(control);
  for (const mode of ['managed', 'delegated']) {
    const fixture = environment(mode);
    const skill = join(fixture.home, '.claude', 'skills', 'uv-fixture');
    mkdirSync(skill, { recursive: true });
    writeFileSync(join(skill, 'SKILL.md'), '---\nname: uv-fixture\ndescription: private runtime fixture\nmetadata:\n  openclaw:\n    requires:\n      bins: [am-uv-fixture]\n    install:\n      - kind: uv\n        package: am-uv-fixture\n        bins: [am-uv-fixture]\n---\nPrivate fixture\n');
    const packageDir = join(fixture.base, 'uv', 'data', 'tools', 'am-uv-fixture');
    if (mode === 'delegated') {
      const scripts = join(packageDir, windows ? 'Scripts' : 'bin');
      const metadata = join(packageDir, windows ? 'Lib/site-packages' : 'lib/python3.13/site-packages', 'am_uv_fixture-1.0.0.dist-info');
      mkdirSync(scripts, { recursive: true }); mkdirSync(metadata, { recursive: true });
      writeFileSync(join(metadata, 'METADATA'), 'Metadata-Version: 2.4\nName: am-uv-fixture\nVersion: 1.0.0\n\n');
      const name = windows ? 'am-uv-fixture.exe' : 'am-uv-fixture';
      const source = join(scripts, name), entry = join(fixture.bin, name);
      copyFileSync(copied, source);
      if (windows) copyFileSync(source, entry);
      else { chmodSync(source, 0o755); symlinkSync(source, entry); }
      writeFileSync(join(packageDir, 'uv-receipt.toml'), `[tool]\nentrypoints = [{ name = "am-uv-fixture", install-path = ${JSON.stringify(entry)} }]\n`);
      fixture.env.UV_TOOL_DIR = join(fixture.base, 'uv', 'data', 'tools');
      fixture.env.UV_TOOL_BIN_DIR = fixture.bin;
    }
    const originalPackage = mode === 'delegated' ? tree(packageDir) : [];
    const originalBin = tree(fixture.bin);
    const status = JSON.parse((await run(engine, ['programs:status', '--json'], fixture)).stdout);
    assert.equal(status.ok, true, JSON.stringify(status));
    const commands = status.data.elements.find(element => element.program === 'pypi:am-uv-fixture').commands;
    for (const action of ['install', 'upgrade']) assert(commands[action].includes('--no-python-downloads'), `Status ${action} omits the runtime boundary`);
    const args = ['--program', 'pypi:am-uv-fixture', '--action', mode === 'managed' ? 'install' : 'upgrade', '--version', '2.0.0',
      ...(mode === 'managed' ? ['--entry-dir', '~/bin'] : [])];
    const preview = JSON.parse((await run(engine, ['programs:preview', ...args, '--json'], fixture)).stdout);
    assert.equal(preview.ok, true, JSON.stringify(preview));
    assert(!existsSync(fixture.env.AGENT_MANAGER_DATA_DIR), 'Preview created manager state');
    assert.deepEqual(tree(fixture.bin), originalBin);
    const command = preview.data.manual.find(command => command.includes('--no-python-downloads'));
    assert(command, 'The copyable uv command omits the runtime boundary');
    const parked = join(fixture.base, 'parked-package');
    if (mode === 'delegated') renameSync(packageDir, parked);
    const before = requests.length;
    const shell = windows ? join(process.env.SystemRoot, 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe') : '/bin/sh';
    for (const copiedCommand of [command, commands.install]) {
      const manual = await run(shell, windows ? ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', copiedCommand] : ['-c', copiedCommand], fixture);
      assert.notEqual(manual.status, 0);
      assert(`${manual.stdout}${manual.stderr}`.toLowerCase().includes('python'), manual.stderr);
      assert.equal(requests.length, before, 'Copied command attempted a Python download');
    }
    if (mode === 'delegated') { assert(!existsSync(packageDir)); renameSync(parked, packageDir); }
    const applied = JSON.parse((await run(engine, ['programs:apply', ...args, '--json'], fixture)).stdout);
    assert.equal(applied.ok, false, JSON.stringify(applied));
    assert(JSON.stringify(applied.errors).toLowerCase().includes('python'), JSON.stringify(applied));
    assert.equal(requests.length, before, 'Agent Manager attempted a Python download');
    noRuntime(fixture);
    assert.deepEqual(tree(fixture.bin), originalBin, 'Original entry was not restored');
    if (mode === 'delegated') assert.deepEqual(tree(packageDir), originalPackage, 'Original package was not restored');
  }
  console.log(`Native ${process.platform} ${version.stdout.trim()}: download control observed; managed/delegated CLI and copied commands blocked Python downloads and restored the original delegated files.`);
  complete = true;
} finally {
  await new Promise(resolve => server.close(resolve));
  if (complete) rmSync(root, { recursive: true });
  else console.error(`Retained private uv failure fixture: ${root}`);
}
