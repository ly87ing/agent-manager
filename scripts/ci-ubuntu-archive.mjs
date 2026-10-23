import { execFileSync } from 'node:child_process';
import { lstatSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

function archiveUri(uri) {
  return uri.replace(/^https?:\/\/azure\.archive\.ubuntu\.com\/ubuntu(\/?)$/, 'https://archive.ubuntu.com/ubuntu$1');
}

function uriTokens(text) {
  const comment = text.indexOf('#');
  const end = comment < 0 ? text.length : comment;
  return text.slice(0, end).replace(/\S+/g, archiveUri) + text.slice(end);
}

export function rewriteUbuntuArchive(text, format) {
  if (!['list', 'sources', 'mirrors'].includes(format)) throw new Error('Expected an APT list, sources or mirrors format');
  let uris = false;
  return text.split('\n').map(line => {
    if (/^[ \t]*#/.test(line)) return line;
    if (format !== 'sources') {
      const legacy = format === 'list'
        ? line.match(/^([ \t]*deb(?:-src)?[ \t]+(?:\[[^\]\r\n]*\][ \t]+)?)(\S+)/)
        : line.match(/^([ \t]*)(\S+)/);
      return legacy ? legacy[1] + archiveUri(legacy[2]) + line.slice(legacy[0].length) : line;
    }
    if (/^[ \t]*\r?$/.test(line)) uris = false;
    const field = line.match(/^URIs:[ \t]*/i);
    if (field) {
      uris = true;
      return field[0] + uriTokens(line.slice(field[0].length));
    }
    if (/^\S/.test(line)) uris = false;
    return uris && /^[ \t]/.test(line) ? uriTokens(line) : line;
  }).join('\n');
}

function main() {
  if (process.platform !== 'linux' || process.env.CI !== 'true'
      || process.env.GITHUB_ACTIONS !== 'true' || process.env.RUNNER_OS !== 'Linux'
      || process.env.RUNNER_ENVIRONMENT !== 'github-hosted') {
    throw new Error('Ubuntu archive preparation requires a GitHub-hosted Linux CI runner');
  }
  let found = false;
  for (const [file, format] of [
    ['/etc/apt/sources.list', 'list'],
    ['/etc/apt/sources.list.d/ubuntu.sources', 'sources'],
    ['/etc/apt/apt-mirrors.txt', 'mirrors'],
  ]) {
    let before;
    try {
      if (!lstatSync(file).isFile()) throw new Error(`Expected an ordinary APT source file: ${file}`);
      before = readFileSync(file, 'utf8');
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
      console.log(`APT source absent: ${file}`);
      continue;
    }
    found = true;
    const after = rewriteUbuntuArchive(before, format);
    if (after === before) {
      console.log(`APT source unchanged: ${file}`);
      continue;
    }
    if (readFileSync(file, 'utf8') !== before) throw new Error(`APT source changed during preparation: ${file}`);
    execFileSync('sudo', ['-n', '/usr/bin/tee', '--', file], { input: after, stdio: ['pipe', 'ignore', 'pipe'] });
    if (readFileSync(file, 'utf8') !== after) throw new Error(`APT source readback failed: ${file}`);
    console.log(`APT archive URI verified: ${file}`);
  }
  if (!found) throw new Error('No standard Ubuntu APT source file found');
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
