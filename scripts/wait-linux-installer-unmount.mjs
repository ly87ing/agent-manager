// Read-only barrier before removing an owned CI fixture; never unmounts anything.
import { readFile } from 'node:fs/promises';
import { posix } from 'node:path';
import { pathToFileURL } from 'node:url';
import { setTimeout } from 'node:timers/promises';

function mountPath(value) {
  if (/\\(?!040|011|012|134)/.test(value)) throw new Error('Unknown mount-path escape');
  const decoded = value.replace(/\\(040|011|012|134)/g, (_, octal) => String.fromCharCode(parseInt(octal, 8)));
  if (!posix.isAbsolute(decoded)) throw new Error('Mount path is not absolute');
  return posix.normalize(decoded);
}

export function mountsBelowFixture(text, root) {
  if (!posix.isAbsolute(root)) throw new Error('Fixture path must be absolute');
  root = posix.normalize(root);
  if (!text.trim()) throw new Error('Mount table is empty');
  const matches = [];
  for (const line of text.trimEnd().split('\n')) {
    const parts = line.split(' - ');
    const fields = parts[0].split(' ');
    if (parts.length !== 2 || fields.length < 6 || parts[1].split(' ').length < 3
      || !/^\d+$/.test(fields[0]) || !/^\d+$/.test(fields[1]) || !/^\d+:\d+$/.test(fields[2])) {
      throw new Error('Mount table could not be verified');
    }
    const mounted = mountPath(fields[4]);
    if (mounted === root || mounted.startsWith(root === '/' ? '/' : `${root}/`)) matches.push(mounted);
  }
  return matches;
}

export async function waitForFixtureUnmounts(root, {
  readMountInfo = async () => new TextDecoder('utf-8', { fatal: true }).decode(await readFile('/proc/self/mountinfo')),
  pause = () => setTimeout(100),
} = {}) {
  let observed = 0;
  for (let poll = 0; poll <= 50; poll++) {
    const mounts = mountsBelowFixture(await readMountInfo(), root);
    observed = Math.max(observed, mounts.length);
    if (mounts.length === 0) return { mountsObserved: observed, polls: poll + 1, status: 'ready' };
    if (poll < 50) await pause();
  }
  throw new Error('Installer fixture still has active mounts; retain the temporary directory');
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    if (process.platform !== 'linux' || process.argv.length !== 3) throw new Error('Expected one Linux fixture directory');
    console.log(JSON.stringify(await waitForFixtureUnmounts(process.argv[2])));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
