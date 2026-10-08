import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';

const tag = process.env.RELEASE_TAG;
const repo = process.env.GITHUB_REPOSITORY;
if (!tag || !repo) {
  console.error('RELEASE_TAG or GITHUB_REPOSITORY is empty');
  process.exit(1);
}
const version = tag.replace(/^v/, '');
const dir = 'updater-assets';
const files = fs.readdirSync(dir).filter((name) => !['latest.json', 'SHA256SUMS.txt'].includes(name));

const findOne = (pred, desc) => {
  const hits = files.filter(pred);
  if (hits.length !== 1) {
    console.error(`expected exactly 1 ${desc}, found ${hits.length}: ${hits.join(', ')}`);
    process.exit(1);
  }
  return hits[0];
};
const entry = (bundle) => {
  const sig = `${bundle}.sig`;
  if (!files.includes(sig)) {
    console.error(`missing updater signature ${sig}`);
    process.exit(1);
  }
  const signature = fs.readFileSync(path.join(dir, sig), 'utf8').trim();
  if (!signature || fs.statSync(path.join(dir, bundle)).size === 0) {
    throw new Error(`empty updater bundle or signature: ${bundle}`);
  }
  return {
    signature,
    url: `https://github.com/${repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(bundle)}`,
  };
};

const macArm = findOne((f) => f.endsWith('.app.tar.gz') && f.includes('aarch64'), 'macOS aarch64 updater bundle');
const macX64 = findOne((f) => f.endsWith('.app.tar.gz') && !f.includes('aarch64') && (f.includes('x86_64') || f.includes('x64')), 'macOS x86_64 updater bundle');
const appimage = findOne((f) => f.endsWith('.AppImage'), 'Linux AppImage updater bundle');
const msi = findOne((f) => f.endsWith('.msi'), 'Windows msi updater bundle');
const nsis = findOne((f) => f.endsWith('-setup.exe'), 'Windows nsis updater bundle');

const eMacArm = entry(macArm);
const eMacX64 = entry(macX64);
const eLinux = entry(appimage);
const eMsi = entry(msi);
const eNsis = entry(nsis);

// 与 tauri-action uploadVersionJSON 默认行为一致（updaterJsonPreferNsis=false）：
// windows-x86_64 主键取 msi；每个平台同时写 {os}-{arch}-{bundle} 键。
const latest = {
  version,
  notes: `Agent Manager ${tag}`,
  pub_date: new Date().toISOString(),
  platforms: {
    'darwin-aarch64': eMacArm,
    'darwin-aarch64-app': eMacArm,
    'darwin-x86_64': eMacX64,
    'darwin-x86_64-app': eMacX64,
    'linux-x86_64': eLinux,
    'linux-x86_64-appimage': eLinux,
    'windows-x86_64': eMsi,
    'windows-x86_64-msi': eMsi,
    'windows-x86_64-nsis': eNsis,
  },
};
fs.writeFileSync('latest.json', JSON.stringify(latest, null, 2) + '\n');
// 使用从 Release 下载后的真实文件名；GitHub 会规范化上传时的空格等字符。
const checksum = (file) => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const sums = files.sort().map((name) => `${checksum(path.join(dir, name))}  ${name}`);
sums.push(`${checksum('latest.json')}  latest.json`);
fs.writeFileSync('SHA256SUMS.txt', sums.join('\n') + '\n');
console.log(`latest.json platforms: ${Object.keys(latest.platforms).join(', ')}`);
