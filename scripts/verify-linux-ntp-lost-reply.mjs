// CI-only service fault injection; the product never changes daemon environment/lifecycle.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { existsSync, readFileSync, lstatSync } from 'node:fs';
import { resolve } from 'node:path';

export async function lostReply({ run, rootNode, cli, repair, native, generated, selected, targetFile, env }) {
  assert.equal(process.env.CI, 'true');
  assert.equal(process.env.GITHUB_ACTIONS, 'true');
  assert.equal(process.env.AM_ISOLATED_LINUX_NTP, '1');
  const root = `/run/agent-manager-ntp-ci-${randomUUID()}`;
  const library = `${root}/lost-reply.so`;
  const directory = '/run/systemd/system/systemd-timesyncd.service.d';
  const path = `${directory}/90-agent-manager-ci.conf`;
  const hadDirectory = existsSync(directory);
  const text = `[Service]\nEnvironment=LD_PRELOAD=${library}\n`;
  const keys = ['LANG', 'LC_ALL', 'LC_CTYPE', 'LC_COLLATE', 'LC_MESSAGES', 'LC_MONETARY', 'LC_NUMERIC', 'LC_TIME'];
  const managerArgs = ['--user', '--auto-start=no', '--json=short', 'call', 'org.freedesktop.systemd1', '/org/freedesktop/systemd1'];
  const userBus = (iface, method, ...args) => run('/usr/bin/busctl', [...managerArgs, iface, method, ...args], false, env);
  function values() {
    const variant = JSON.parse(userBus('org.freedesktop.DBus.Properties', 'Get', 'ss', 'org.freedesktop.systemd1.Manager', 'Environment').stdout).data[0];
    assert.equal(variant.type, 'as');
    return variant.data.filter(value => keys.includes(value.split('=')[0]));
  }
  const originalVariables = values();
  assert(!existsSync(path), 'Refusing an existing service fixture');
  assert.equal(readFileSync(targetFile, 'utf8'), generated(selected));
  let prepared = false;
  try {
    rootNode("import fs from 'node:fs';const [root,dir,path,text,target,expected]=process.argv.slice(1);if(fs.readFileSync(target,'utf8')!==expected)throw Error('Changed owned NTP file');fs.unlinkSync(target);fs.mkdirSync(root,{mode:0o755});fs.mkdirSync(dir,{recursive:true,mode:0o755});fs.writeFileSync(path,text,{flag:'wx',mode:0o644});", [root, directory, path, text, targetFile, generated(selected)]);
    prepared = true;
    run('/usr/bin/sudo', ['-n', '/usr/bin/cc', '-shared', '-fPIC', '-Wall', '-Wextra', '-Werror', '-o', library,
      resolve('engine-rs/tests/support/linux_ntp_lost_reply.c'), '-lsystemd', '-ldl']);
    rootNode("import fs from 'node:fs';const p=process.argv[1];if(!fs.lstatSync(p).isFile())throw Error('Missing owned test library');fs.chmodSync(p,0o644);", [library]);
    assert.equal(lstatSync(library).uid, 0);
    run('/usr/bin/sudo', ['-n', '/usr/bin/systemctl', 'daemon-reload']);
    run('/usr/bin/sudo', ['-n', '/usr/bin/systemctl', 'restart', 'systemd-timesyncd.service']);
    const before = native();
    assert.deepEqual(before.RuntimeNTPServers, []);
    const pid = before.pid;
    // Establish that the actual vendor process loaded only our owned test shim.
    const maps = rootNode(String.raw`import fs from 'node:fs';const [pid,lib]=process.argv.slice(1);if(!fs.readFileSync('/proc/'+pid+'/maps','utf8').split('\n').some(line=>line.endsWith(' '+lib)))throw Error('Native daemon did not load CI reply fault');`, [String(pid), library]);
    assert.equal(maps.status, 0);
    userBus('org.freedesktop.systemd1.Manager', 'SetEnvironment', 'as', String(keys.length), ...keys.map(key => `${key}=C`));
    const profile = JSON.stringify({ ntp_server: selected, locale: 'C.UTF-8' });
    const preview = cli('environment:repair-preview', [], false, profile).data;
    assert(preview.items.filter(item => item.scope === 'session').every(item => item.status === 'change'));
    const result = await repair('ntp-allow', profile);
    assert.equal(result.status, 'partial', JSON.stringify(result.items));
    assert.equal(result.items.find(item => item.id === 'ntp.server').status, 'verified_after_error');
    assert(result.items.filter(item => item.scope === 'session').every(item => item.status === 'not_applied'));
    assert.deepEqual(values().sort(), keys.map(key => `${key}=C`).sort(), 'A partial NTP repair continued into user-manager writes');
    assert.equal(readFileSync(targetFile, 'utf8'), generated(selected));
    assert.deepEqual(native(), { ...before, RuntimeNTPServers: [selected] }, 'Lost-reply repair changed unrelated native state');
    console.log('Native vendor NTP lost setter reply: both actual channels read back, partial retained, later session writes stopped.');
  } finally {
    userBus('org.freedesktop.systemd1.Manager', 'UnsetAndSetEnvironment', 'asas', String(keys.length), ...keys, String(originalVariables.length), ...originalVariables);
    assert.deepEqual(values().sort(), originalVariables.sort());
    if (prepared) {
      rootNode("import fs from 'node:fs';const [path,text,root,lib,dir,had]=process.argv.slice(1);if(fs.readFileSync(path,'utf8')!==text)throw Error('Changed service fixture');fs.unlinkSync(path);if(fs.existsSync(lib))fs.unlinkSync(lib);fs.rmdirSync(root);if(had==='0'){if(fs.readdirSync(dir).length)throw Error('Unexpected service fixtures');fs.rmdirSync(dir);}", [path, text, root, library, directory, hadDirectory ? '1' : '0']);
      run('/usr/bin/sudo', ['-n', '/usr/bin/systemctl', 'daemon-reload']);
      run('/usr/bin/sudo', ['-n', '/usr/bin/systemctl', 'restart', 'systemd-timesyncd.service']);
    }
  }
}
