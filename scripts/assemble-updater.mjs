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

// Match architecture tokens, not arbitrary substrings in product names.
const architecture = (name) => {
  const suffix = ['.app.tar.gz', '.AppImage', '.deb', '.msi', '-setup.exe'].find(suffix => name.endsWith(suffix));
  if (!suffix) return null;
  const stem = name.slice(0, -suffix.length);
  const locale = suffix === '.msi' ? '(?:_[A-Za-z]{2,3}(?:-[A-Za-z0-9]{2,8})*)?' : '';
  const token = stem.match(new RegExp(`(?:^|[_.-])(x86_64|x64|amd64|aarch64|arm64)${locale}$`))?.[1];
  const normalize = token => ['aarch64', 'arm64'].includes(token) ? 'aarch64' : 'x86_64';
  const mentioned = [...stem.matchAll(/(?:^|[_.-])(x86_64|x64|amd64|aarch64|arm64)(?=$|[_.-])/g)].map(match => normalize(match[1]));
  if (!token || new Set(mentioned).size !== 1) throw new Error(`missing, unknown or conflicting bundle architecture: ${name}`);
  const arch = normalize(token);
  if (suffix === '.msi' && arch !== 'x86_64') throw new Error(`unsupported ARM64 MSI updater bundle: ${name}`);
  return arch;
};
const architectures = new Map(files.map(name => [name, architecture(name)]));

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

const bundle = (suffix, arch) => findOne(f => f.endsWith(suffix) && architectures.get(f) === arch, `${arch} ${suffix} updater bundle`);
const macArm = bundle('.app.tar.gz', 'aarch64');
const macX64 = bundle('.app.tar.gz', 'x86_64');
const appimage = bundle('.AppImage', 'x86_64');
const deb = bundle('.deb', 'x86_64');
const msi = bundle('.msi', 'x86_64');
const nsis = bundle('-setup.exe', 'x86_64');
const appimageArm = bundle('.AppImage', 'aarch64');
const debArm = bundle('.deb', 'aarch64');
const nsisArm = bundle('-setup.exe', 'aarch64');

const eMacArm = entry(macArm);
const eMacX64 = entry(macX64);
const eLinux = entry(appimage);
const eDeb = entry(deb);
const eMsi = entry(msi);
const eNsis = entry(nsis);
const eLinuxArm = entry(appimageArm);
const eDebArm = entry(debArm);
const eNsisArm = entry(nsisArm);

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
    'linux-x86_64-deb': eDeb,
    'linux-aarch64': eLinuxArm,
    'linux-aarch64-appimage': eLinuxArm,
    'linux-aarch64-deb': eDebArm,
    'windows-x86_64': eMsi,
    'windows-x86_64-msi': eMsi,
    'windows-x86_64-nsis': eNsis,
    'windows-aarch64': eNsisArm,
    'windows-aarch64-nsis': eNsisArm,
  },
};
fs.writeFileSync('latest.json', JSON.stringify(latest, null, 2) + '\n');
// 使用从 Release 下载后的真实文件名；GitHub 会规范化上传时的空格等字符。
const checksum = (file) => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const sums = files.sort().map((name) => `${checksum(path.join(dir, name))}  ${name}`);
sums.push(`${checksum('latest.json')}  latest.json`);
fs.writeFileSync('SHA256SUMS.txt', sums.join('\n') + '\n');
console.log(`latest.json platforms: ${Object.keys(latest.platforms).join(', ')}`);
