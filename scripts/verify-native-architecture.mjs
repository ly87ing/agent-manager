import assert from 'node:assert/strict';
import { appendFileSync, readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

const targets = {
  'x86_64-apple-darwin': ['darwin', 'x64', 'macho', 0x01000007],
  'aarch64-apple-darwin': ['darwin', 'arm64', 'macho', 0x0100000c],
  'x86_64-unknown-linux-gnu': ['linux', 'x64', 'elf', 62],
  'aarch64-unknown-linux-gnu': ['linux', 'arm64', 'elf', 183],
  'x86_64-pc-windows-msvc': ['win32', 'x64', 'pe', 0x8664],
  'aarch64-pc-windows-msvc': ['win32', 'arm64', 'pe', 0xaa64],
};

export function verifyHost(platform, arch, runnerOS, runnerArch, rustHost, expected) {
  const target = Object.keys(targets).find(key => targets[key][0] === platform && targets[key][1] === arch);
  assert.ok(target, 'Unsupported native build platform or architecture');
  assert.equal({ macOS: 'darwin', Linux: 'linux', Windows: 'win32' }[runnerOS], platform, 'Node platform differs from runner');
  assert.equal({ X64: 'x64', ARM64: 'arm64' }[runnerArch], arch, 'Node architecture differs from runner');
  assert.equal(rustHost, target, 'Rust host is not native to the runner');
  if (expected) assert.equal(target, expected, 'Build target is not native to the runner');
  return target;
}

export function verifyExecutable(bytes, target) {
  assert.ok(Object.hasOwn(targets, target), 'Unsupported executable target');
  assert.ok(bytes.length >= 64, 'Truncated executable header');
  const [, , format, machine] = targets[target];
  if (format === 'elf') {
    assert.equal(bytes.subarray(0, 7).toString('hex'), '7f454c46020101', 'Expected little-endian ELF64');
    assert.equal(bytes.readUInt16LE(18), machine, 'ELF architecture mismatch');
  } else if (format === 'pe') {
    assert.equal(bytes.subarray(0, 2).toString(), 'MZ', 'Expected PE executable');
    const offset = bytes.readUInt32LE(0x3c);
    assert.ok(offset >= 64 && offset + 24 <= bytes.length, 'Invalid PE header offset');
    assert.equal(bytes.readUInt32LE(offset), 0x4550, 'Invalid PE signature');
    assert.equal(bytes.readUInt16LE(offset + 4), machine, 'PE architecture mismatch');
  } else {
    assert.equal(bytes.readUInt32LE(0), 0xfeedfacf, 'Expected little-endian Mach-O 64');
    assert.equal(bytes.readUInt32LE(4), machine, 'Mach-O architecture mismatch');
  }
  return target;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [mode, target, file] = process.argv.slice(2);
  if (mode === 'host' && !file) {
    const rustHost = execFileSync('rustc', ['-vV'], { encoding: 'utf8' }).match(/^host: (\S+)$/m)?.[1];
    const native = verifyHost(process.platform, process.arch, process.env.RUNNER_OS, process.env.RUNNER_ARCH, rustHost, target);
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `target=${native}\n`);
    console.log(`Native runner, Node and Rust agree: ${native}`);
  } else if (mode === 'binary' && target && file && process.argv.length === 5) {
    verifyExecutable(readFileSync(file), target);
    console.log(`Executable architecture verified: ${target}`);
  } else {
    throw new Error('Usage: verify-native-architecture.mjs host [target] | binary <target> <file>');
  }
}
