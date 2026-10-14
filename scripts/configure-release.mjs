import { readFileSync, writeFileSync } from 'node:fs';

const path = 'src-tauri/tauri.conf.json';
const config = JSON.parse(readFileSync(path, 'utf8'));
const tag = process.env.RELEASE_TAG || '';
if (!/^v[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc)\.[0-9]+)?$/.test(tag)) {
  console.error('RELEASE_TAG must match vX.Y.Z or vX.Y.Z-(alpha|beta|rc).N');
  process.exit(1);
}
const version = tag.slice(1);
if (config.version !== version.split('-')[0] && config.version !== version) {
  console.error('Release tag does not match the source configuration version');
  process.exit(1);
}
const pubkey = process.env.TAURI_UPDATER_PUBKEY || '';
if (!pubkey || pubkey.startsWith('YOUR_')) {
  console.error('TAURI_UPDATER_PUBKEY is empty or a YOUR_* placeholder');
  process.exit(1);
}

// Tauri derives its runtime updater version from this ephemeral configuration.
config.version = version;
config.bundle = config.bundle || {};
config.bundle.createUpdaterArtifacts = true;
config.plugins = config.plugins || {};
config.plugins.updater = config.plugins.updater || {};
config.plugins.updater.pubkey = pubkey;
if (process.platform === 'darwin' && process.env.APPLE_SIGNING_ENABLED !== 'true') {
  config.bundle.macOS = { ...config.bundle.macOS, signingIdentity: '-' };
}
if (process.platform === 'win32' && process.env.WINDOWS_SIGNING_ENABLED === 'true') {
  const thumbprint = process.env.WINDOWS_CERTIFICATE_THUMBPRINT || '';
  const timestampUrl = process.env.WINDOWS_TIMESTAMP_URL || '';
  if (!thumbprint || !timestampUrl) {
    console.error('WINDOWS_CERTIFICATE_THUMBPRINT or WINDOWS_TIMESTAMP_URL is empty');
    process.exit(1);
  }
  config.bundle.windows = config.bundle.windows || {};
  config.bundle.windows.certificateThumbprint = thumbprint;
  config.bundle.windows.digestAlgorithm = 'sha256';
  config.bundle.windows.timestampUrl = timestampUrl;
}
writeFileSync(path, JSON.stringify(config, null, 2) + '\n');
