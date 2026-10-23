import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { rewriteUbuntuArchive } from './ci-ubuntu-archive.mjs';

test('legacy sources preserve signing options, suites, components and comments', () => {
  const before = '# http://azure.archive.ubuntu.com/ubuntu/\n'
    + 'deb [arch=amd64 signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] http://azure.archive.ubuntu.com/ubuntu/ jammy main universe # retain\n'
    + '\tdeb-src https://azure.archive.ubuntu.com/ubuntu jammy-updates main\n'
    + 'deb http://security.ubuntu.com/ubuntu jammy-security main\n';
  const expected = '# http://azure.archive.ubuntu.com/ubuntu/\n'
    + 'deb [arch=amd64 signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] https://archive.ubuntu.com/ubuntu/ jammy main universe # retain\n'
    + '\tdeb-src https://archive.ubuntu.com/ubuntu jammy-updates main\n'
    + 'deb http://security.ubuntu.com/ubuntu jammy-security main\n';
  assert.equal(rewriteUbuntuArchive(before, 'list'), expected);
  assert.equal(rewriteUbuntuArchive(expected, 'list'), expected);
});

test('deb822 rewrites URI tokens and continuations while preserving every other field', () => {
  const before = 'Types: deb deb-src\r\n'
    + 'URIs: http://azure.archive.ubuntu.com/ubuntu/ https://azure.archive.ubuntu.com/ubuntu\r\n'
    + ' https://YOUR_MIRROR/ubuntu http://azure.archive.ubuntu.com/ubuntu/ # http://azure.archive.ubuntu.com/ubuntu/\r\n'
    + 'Suites: noble noble-updates\r\nComponents: main universe\r\n'
    + 'Signed-By:\r\n http://azure.archive.ubuntu.com/ubuntu/\r\n'
    + 'X-Comment: keep source examples\r\n deb http://azure.archive.ubuntu.com/ubuntu jammy main\r\n'
    + '\r\n# keep this URI: http://azure.archive.ubuntu.com/ubuntu/\r\n'
    + 'URIs: http://security.ubuntu.com/ubuntu/\r\nSuites: noble-security\r\n';
  const expected = 'Types: deb deb-src\r\n'
    + 'URIs: https://archive.ubuntu.com/ubuntu/ https://archive.ubuntu.com/ubuntu\r\n'
    + ' https://YOUR_MIRROR/ubuntu https://archive.ubuntu.com/ubuntu/ # http://azure.archive.ubuntu.com/ubuntu/\r\n'
    + 'Suites: noble noble-updates\r\nComponents: main universe\r\n'
    + 'Signed-By:\r\n http://azure.archive.ubuntu.com/ubuntu/\r\n'
    + 'X-Comment: keep source examples\r\n deb http://azure.archive.ubuntu.com/ubuntu jammy main\r\n'
    + '\r\n# keep this URI: http://azure.archive.ubuntu.com/ubuntu/\r\n'
    + 'URIs: http://security.ubuntu.com/ubuntu/\r\nSuites: noble-security\r\n';
  assert.equal(rewriteUbuntuArchive(before, 'sources'), expected);
  assert.equal(rewriteUbuntuArchive(expected, 'sources'), expected);
});

test('lookalikes, subpaths, ports, ARM archives and third-party sources remain unchanged', () => {
  const uris = [
    'http://azure.archive.ubuntu.com/ubuntu-extra',
    'http://azure.archive.ubuntu.com/ubuntu/pool',
    'http://azure.archive.ubuntu.com/ubuntu?mirror=1',
    'http://azure.archive.ubuntu.com:80/ubuntu',
    'http://azure.archive.ubuntu.com.YOUR_HOST/ubuntu',
    'http://ports.ubuntu.com/ubuntu-ports/',
    'https://YOUR_MIRROR/ubuntu/',
  ];
  for (const uri of uris) {
    assert.equal(rewriteUbuntuArchive(`deb ${uri} jammy main\n`, 'list'), `deb ${uri} jammy main\n`);
    assert.equal(rewriteUbuntuArchive(`URIs: ${uri}\nSuites: noble\n`, 'sources'), `URIs: ${uri}\nSuites: noble\n`);
    assert.equal(rewriteUbuntuArchive(`${uri}\tpriority:1\n`, 'mirrors'), `${uri}\tpriority:1\n`);
  }
  for (const format of ['list', 'sources', 'mirrors']) assert.equal(rewriteUbuntuArchive('', format), '');
});

test('hosted runner mirror lists preserve priorities, fallbacks and source-file pointers', () => {
  const before = '# http://azure.archive.ubuntu.com/ubuntu/\n'
    + 'http://azure.archive.ubuntu.com/ubuntu/\tpriority:1\n'
    + 'https://archive.ubuntu.com/ubuntu/\tpriority:2\n'
    + 'https://security.ubuntu.com/ubuntu/\tpriority:3\n'
    + ' http://ports.ubuntu.com/ubuntu-ports/\tpriority:4\n';
  const expected = '# http://azure.archive.ubuntu.com/ubuntu/\n'
    + 'https://archive.ubuntu.com/ubuntu/\tpriority:1\n'
    + 'https://archive.ubuntu.com/ubuntu/\tpriority:2\n'
    + 'https://security.ubuntu.com/ubuntu/\tpriority:3\n'
    + ' http://ports.ubuntu.com/ubuntu-ports/\tpriority:4\n';
  assert.equal(rewriteUbuntuArchive(before, 'mirrors'), expected);
  assert.equal(rewriteUbuntuArchive(expected, 'mirrors'), expected);
  const pointer = 'mirror+file:/etc/apt/apt-mirrors.txt';
  assert.equal(rewriteUbuntuArchive(`URIs: ${pointer}\n`, 'sources'), `URIs: ${pointer}\n`);
  assert.equal(rewriteUbuntuArchive(`deb ${pointer} jammy main\n`, 'list'), `deb ${pointer} jammy main\n`);
});

test('direct use outside hosted Linux CI refuses before privileged preparation', () => {
  for (const refused of [
    { CI: 'false' }, { GITHUB_ACTIONS: 'false' },
    { RUNNER_OS: 'Windows' }, { RUNNER_ENVIRONMENT: 'self-hosted' },
  ]) {
    const result = spawnSync(process.execPath, [fileURLToPath(new URL('./ci-ubuntu-archive.mjs', import.meta.url))], {
      encoding: 'utf8', timeout: 5000,
      env: { ...process.env, CI: 'true', GITHUB_ACTIONS: 'true', RUNNER_OS: 'Linux', RUNNER_ENVIRONMENT: 'github-hosted', ...refused },
    });
    assert.equal(result.status, 1);
    assert.equal(result.stdout, '');
    assert.match(result.stderr, /^Ubuntu archive preparation requires a GitHub-hosted Linux CI runner\r?\n$/);
  }
});
