// Existing npm, private loopback registry/tarballs and isolated application state.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { gzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, dirname, isAbsolute, join, resolve } from 'node:path';

assert.equal(process.env.AM_ISOLATED_NATIVE_NPM, '1');
assert(process.env.AM_NATIVE_NPM && isAbsolute(process.env.AM_NATIVE_NPM));
assert(lstatSync(realpathSync(process.env.AM_NATIVE_NPM)).isFile());
const npmBefore = readFileSync(realpathSync(process.env.AM_NATIVE_NPM));
const windows = process.platform === 'win32';
const root = mkdtempSync(join(tmpdir(), 'am npm-native-'));
const home = join(root, 'home'), data = join(root, 'data'), bin = join(home, 'bin');
mkdirSync(bin, { recursive: true });
const config = join(root, 'empty-npmrc'); writeFileSync(config, '');
const globalConfig = join(root, 'empty-global-npmrc'); writeFileSync(globalConfig, '');
const inherited = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(npm_|node_options$|xdg_|appdata$|localappdata$|home$|userprofile$|agent_manager_|https?_proxy$|all_proxy$)/i.test(key)));
const env = { ...inherited, HOME: home, USERPROFILE: home, XDG_CONFIG_HOME: join(home, '.config'), XDG_DATA_HOME: join(home, '.local/share'), XDG_CACHE_HOME: join(home, '.cache'),
  APPDATA: join(home, 'AppData/Roaming'), LOCALAPPDATA: join(home, 'AppData/Local'),
  AGENT_MANAGER_DATA_DIR: data, AGENT_MANAGER_USER_PATH: [bin, dirname(process.env.AM_NATIVE_NPM)].join(delimiter),
  npm_config_userconfig: config, npm_config_globalconfig: globalConfig, npm_config_cache: join(root, 'cache'), npm_config_update_notifier: 'false', NO_PROXY: '127.0.0.1,localhost' };
const engine = resolve(process.env.CARGO_TARGET_DIR || 'engine-rs/target', `debug/agent-manager-engine${windows ? '.exe' : ''}`);

function run(program, args, windowsVerbatimArguments = false) {
  return new Promise((resolve, reject) => {
    const child = spawn(program, args, { env, cwd: root, windowsVerbatimArguments, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    const timeout = setTimeout(() => { child.kill(); reject(Error('Private npm fixture child deadline')); }, 60_000);
    child.stdout.on('data', value => { stdout += value; }); child.stderr.on('data', value => { stderr += value; });
    child.on('error', error => { clearTimeout(timeout); reject(error); });
    child.on('close', code => { clearTimeout(timeout); code === 0 ? resolve(stdout) : reject(Error(`${program}: ${code}\n${stdout}\n${stderr}`)); });
  });
}
async function cli(command, args = []) { const result = JSON.parse(await run(engine, [command, ...args, '--json'])); assert.equal(result.ok, true, JSON.stringify(result)); return result.data; }
function tar(files) {
  const chunks = [];
  for (const [name, text] of files) {
    const bytes = Buffer.from(text), header = Buffer.alloc(512);
    header.write(`package/${name}`, 0, 100); header.write('0000755\0', 100, 8); header.write('0000000\0', 108, 8); header.write('0000000\0', 116, 8);
    header.write(bytes.length.toString(8).padStart(11, '0') + '\0', 124, 12); header.write('00000000000\0', 136, 12); header.fill(32, 148, 156); header.write('0', 156); header.write('ustar\0', 257, 6); header.write('00', 263, 2);
    header.write([...header].reduce((sum, byte) => sum + byte, 0).toString(8).padStart(6, '0') + '\0 ', 148, 8);
    chunks.push(header, bytes, Buffer.alloc((512 - bytes.length % 512) % 512));
  }
  return gzipSync(Buffer.concat([...chunks, Buffer.alloc(1024)]));
}
const packages = [
  { name: 'am-npm-version-fixture', command: 'am-npm-version-fixture', bin: { 'am-npm-version-fixture': './bin/cli.js' }, path: 'bin/cli.js' },
  { name: '@am-fixture/tool-a', command: 'tool-a', bin: './bin/cli.js', path: 'bin/cli.js' },
  { name: 'am-npm-array-fixture', command: 'am-npm-array-fixture', bin: ['bin/am-npm-array-fixture'], path: 'bin/am-npm-array-fixture' },
  { name: 'am-npm-adopt-fixture', command: 'am-npm-adopt-fixture', bin: { 'am-npm-adopt-fixture': 'bin/cli.js' }, path: 'bin/cli.js', adopt: true },
  { name: 'am-npm-direct-fixture', command: 'am-npm-direct-fixture', bin: { 'am-npm-direct-fixture': windows ? 'bin/direct.cmd' : 'bin/direct.sh' }, path: windows ? 'bin/direct.cmd' : 'bin/direct.sh', direct: true },
];
const tarballs = new Map();
for (const [index, pkg] of packages.entries()) for (const version of ['1.0.0', '1.9.0', '2.0.0']) {
  tarballs.set(`/tar/${index}-${version}.tgz`, tar([
    ['package.json', JSON.stringify({ name: pkg.name, version, bin: pkg.bin })],
    [pkg.path, pkg.direct ? (windows ? `@echo off\r\necho ${version}\r\n` : `printf '%s\\n' '${version}'\n`) : `#!/usr/bin/env node\nconsole.log(${JSON.stringify(version)});\n`],
  ]));
}
const unexpected = [];
let registry;
const server = createServer((request, response) => {
  const path = decodeURIComponent(request.url.split('?')[0]);
  if (tarballs.has(path)) { response.end(tarballs.get(path)); return; }
  const index = packages.findIndex(pkg => `/${pkg.name}` === path);
  if (index < 0) { unexpected.push(path); response.writeHead(404); response.end(); return; }
  const pkg = packages[index], versions = {};
  for (const version of ['1.0.0', '1.9.0', '2.0.0']) {
    const route = `/tar/${index}-${version}.tgz`;
    versions[version] = { name: pkg.name, version, bin: pkg.bin, dist: { tarball: registry + route, integrity: 'sha512-' + createHash('sha512').update(tarballs.get(route)).digest('base64') } };
  }
  response.setHeader('Content-Type', 'application/json'); response.end(JSON.stringify({ name: pkg.name, 'dist-tags': { latest: '2.0.0', next: '2.0.0' }, versions }));
});
await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
registry = `http://127.0.0.1:${server.address().port}`;
env.npm_config_registry = registry;
let complete = false;
try {
  for (const pkg of packages) {
    const skill = join(home, '.claude/skills', pkg.command); mkdirSync(skill, { recursive: true });
    writeFileSync(join(skill, 'SKILL.md'), `---\nname: ${pkg.command}\ndescription: private native npm fixture\nmetadata:\n  openclaw:\n    requires:\n      bins: [${pkg.command}]\n    install:\n      - kind: node\n        package: "${pkg.name}"\n        bins: [${pkg.command}]\n---\nPrivate fixture\n`);
    const args = ['--program', `npm:${pkg.name}`, '--entry-dir', bin];
    const output = async () => {
      const entry = join(bin, pkg.command + (windows ? '.cmd' : ''));
      if (!windows) return (await run(entry, [])).trim();
      assert(!/[\r\n%!"&]/.test(entry));
      return (await run(process.env.ComSpec || 'cmd.exe', ['/d', '/v:off', '/s', '/c', `""${entry}""`], true)).trim();
    };
    const verifyOutput = async version => {
      if (pkg.direct && process.platform === 'darwin') {
        // Admission establishes files, not an executable format/interpreter.
        await assert.rejects(output, /ENOEXEC/);
        assert.equal((await run('/bin/sh', [join(bin, pkg.command)])).trim(), version);
      } else assert.equal(await output(), version);
    };
    if (pkg.adopt) {
      const prefix = join(root, 'global prefix');
      const npmCli = windows ? join(dirname(process.env.AM_NATIVE_NPM), 'node_modules/npm/bin/npm-cli.js') : realpathSync(process.env.AM_NATIVE_NPM);
      assert(lstatSync(npmCli).isFile());
      env.AGENT_MANAGER_USER_PATH = [bin, windows ? prefix : join(prefix, 'bin'), dirname(process.env.AM_NATIVE_NPM)].join(delimiter);
      await run(process.execPath, [npmCli, 'install', '--global', '--prefix', prefix, '--no-audit', '--no-fund', '--ignore-scripts', `${pkg.name}@1.0.0`]);
      const source = join(prefix, windows ? 'node_modules' : 'lib/node_modules', pkg.name, 'package.json');
      const before = readFileSync(source);
      await cli('programs:preview', [...args, '--action', 'adopt']); assert.deepEqual(readFileSync(source), before);
      const adopted = await cli('programs:apply', [...args, '--action', 'adopt']);
      assert(!existsSync(source)); assert.deepEqual(readFileSync(join(data, 'programs/backups', adopted.backup, 'package/package.json')), before);
    } else {
      const preview = await cli('programs:preview', [...args, '--action', 'install', '--version', 'v1.0.0']); assert.equal(preview.target_version, '1.0.0');
      if (pkg === packages[0]) assert(!existsSync(data), 'Initial preview wrote app state');
      await cli('programs:apply', [...args, '--action', 'install', '--version', 'v1.0.0']);
    }
    await verifyOutput('1.0.0');
    const range = await cli('programs:preview', [...args, '--action', 'upgrade', '--version', '1']); assert.equal(range.target_version, '1.9.0');
    await cli('programs:apply', [...args, '--action', 'upgrade', '--version', '1']); await verifyOutput('1.9.0');
    const tag = await cli('programs:preview', [...args, '--action', 'upgrade', '--version', 'next']); assert.equal(tag.target_version, '2.0.0');
    await cli('programs:apply', [...args, '--action', 'upgrade', '--version', 'next']); await verifyOutput('2.0.0');
    await cli('programs:apply', [...args, '--action', 'switch', '--version', '1.0.0']); await verifyOutput('1.0.0');
    await cli('programs:apply', [...args, '--action', 'upgrade', '--version', '2.0.0']); await verifyOutput('2.0.0');
    console.log(`Native npm version/entry lifecycle passed: ${pkg.name}`);
  }
  assert.deepEqual(unexpected, []); assert.deepEqual(readFileSync(realpathSync(process.env.AM_NATIVE_NPM)), npmBefore);
  complete = true;
  console.log('Real npm: ordinary/scoped/string/array/direct bins, adoption backup readback, exact v-prefix, range/tag resolution, current selection/reuse and actual entry execution passed; existing npm bytes preserved.');
  if (process.platform === 'darwin') console.log('The macOS no-shebang control retained its expected direct ENOEXEC refusal and required an explicitly selected /bin/sh; file admission is not runtime verification.');
} finally {
  await new Promise(resolve => server.close(resolve));
  if (complete) rmSync(root, { recursive: true, force: true }); else console.error(`Retained private npm fixture: ${root}`);
}
