import test from 'node:test';
import assert from 'node:assert/strict';
import { verifyExecutable, verifyHost } from './verify-native-architecture.mjs';

const cases = [
  ['x86_64-apple-darwin', 'darwin', 'x64', 'macOS', 'X64', 'macho', 0x01000007],
  ['aarch64-apple-darwin', 'darwin', 'arm64', 'macOS', 'ARM64', 'macho', 0x0100000c],
  ['x86_64-unknown-linux-gnu', 'linux', 'x64', 'Linux', 'X64', 'elf', 62],
  ['aarch64-unknown-linux-gnu', 'linux', 'arm64', 'Linux', 'ARM64', 'elf', 183],
  ['x86_64-pc-windows-msvc', 'win32', 'x64', 'Windows', 'X64', 'pe', 0x8664],
  ['aarch64-pc-windows-msvc', 'win32', 'arm64', 'Windows', 'ARM64', 'pe', 0xaa64],
];

function executable(format, machine) {
  const bytes = Buffer.alloc(128);
  if (format === 'elf') {
    Buffer.from('7f454c46020101', 'hex').copy(bytes);
    bytes.writeUInt16LE(machine, 18);
  } else if (format === 'pe') {
    bytes.write('MZ'); bytes.writeUInt32LE(64, 0x3c);
    bytes.writeUInt32LE(0x4550, 64); bytes.writeUInt16LE(machine, 68);
  } else {
    bytes.writeUInt32LE(0xfeedfacf); bytes.writeUInt32LE(machine, 4);
  }
  return bytes;
}

for (const [target, platform, arch, runnerOS, runnerArch, format, machine] of cases) {
  test(`${target}: native host and executable agree`, () => {
    assert.equal(verifyHost(platform, arch, runnerOS, runnerArch, target, target), target);
    assert.equal(verifyExecutable(executable(format, machine), target), target);
    const other = cases.find(row => row[1] === platform && row[2] !== arch)[0];
    assert.throws(() => verifyHost(platform, arch, runnerOS, runnerArch, other, target), /Rust host/);
    assert.throws(() => verifyHost(platform, arch, runnerOS, runnerArch, target, other), /Build target/);
    assert.throws(() => verifyExecutable(executable(format, machine), other), /architecture mismatch/);
  });
}

test('emulated Node, wrong OS and unsupported hosts cannot masquerade as native', () => {
  assert.throws(() => verifyHost('win32', 'x64', 'Windows', 'ARM64', 'x86_64-pc-windows-msvc'), /Node architecture/);
  assert.throws(() => verifyHost('linux', 'arm64', 'macOS', 'ARM64', 'aarch64-unknown-linux-gnu'), /Node platform/);
  assert.throws(() => verifyHost('linux', 'ia32', 'Linux', 'X86', 'i686-unknown-linux-gnu'), /Unsupported/);
});

test('truncated, wrong-format and invalid-offset headers fail before architecture acceptance', () => {
  for (const [target] of cases) assert.throws(() => verifyExecutable(Buffer.alloc(32), target), /Truncated/);
  assert.throws(() => verifyExecutable(Buffer.alloc(128), 'aarch64-unknown-linux-gnu'), /ELF64/);
  assert.throws(() => verifyExecutable(Buffer.alloc(128), 'aarch64-apple-darwin'), /Mach-O/);
  const pe = executable('pe', 0xaa64);
  pe.writeUInt32LE(0xffffffff, 0x3c);
  assert.throws(() => verifyExecutable(pe, 'aarch64-pc-windows-msvc'), /PE header offset/);
  assert.throws(() => verifyExecutable(Buffer.alloc(128), 'unsupported'), /Unsupported/);
});
