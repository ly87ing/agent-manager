import test from 'node:test';
import assert from 'node:assert/strict';
import { mountsBelowFixture, waitForFixtureUnmounts } from './wait-linux-installer-unmount.mjs';

const base = '1 1 8:1 / / rw - ext4 /dev/YOUR_ROOT rw\n';
const doc = '2 1 0:50 / /owned/runtime/xdg/doc rw shared:4 future:1 - fuse.portal portal rw\n';

test('matches nested FUSE and same-device bind mounts, including the fixture root', () => {
  const bind = '3 1 8:1 /source /owned/bind rw - ext4 /dev/YOUR_ROOT rw\n';
  assert.deepEqual(mountsBelowFixture(base + doc + bind, '/owned'), ['/owned/runtime/xdg/doc', '/owned/bind']);
  assert.deepEqual(mountsBelowFixture(base + doc, '/owned/runtime/xdg/doc'), ['/owned/runtime/xdg/doc']);
  assert.deepEqual(mountsBelowFixture(base + doc, '/own'), []);
});

test('decodes kernel mount-path escapes once without confusing sibling prefixes', () => {
  const escaped = '2 1 0:50 / /owned\\040space/runtime\\134040/doc rw - fuse.portal portal rw\n';
  assert.deepEqual(mountsBelowFixture(base + escaped, '/owned space'), ['/owned space/runtime\\040/doc']);
  const controls = '2 1 0:50 / /owned\\011tab/child\\012line rw - fuse.portal portal rw\n';
  assert.deepEqual(mountsBelowFixture(base + controls, '/owned\ttab'), ['/owned\ttab/child\nline']);
});

test('unknown mount evidence is rejected rather than treated as unmounted', () => {
  for (const text of ['', 'bad table', '1 1 bad / /owned rw - ext4 root rw', base + doc.replace('/owned/', '/bad\\777/')]) {
    assert.throws(() => mountsBelowFixture(text, '/owned'));
  }
  assert.throws(() => mountsBelowFixture(base, 'relative'));
});

test('waits for an owned portal mount to disappear before allowing cleanup', async () => {
  const snapshots = [base + doc, base + doc, base];
  let pauses = 0;
  const result = await waitForFixtureUnmounts('/owned', {
    readMountInfo: async () => snapshots.shift(), pause: async () => { pauses++; },
  });
  assert.deepEqual(result, { mountsObserved: 1, polls: 3, status: 'ready' });
  assert.equal(pauses, 2);
});

test('a retained mount or unreadable table never authorizes cleanup', async () => {
  let cleanup = false;
  let pauses = 0;
  await assert.rejects(waitForFixtureUnmounts('/owned', {
    readMountInfo: async () => base + doc, pause: async () => { pauses++; },
  }).then(() => { cleanup = true; }), /retain the temporary directory/);
  assert.equal(pauses, 50);
  assert.equal(cleanup, false);
  await assert.rejects(waitForFixtureUnmounts('/owned', { readMountInfo: async () => { throw new Error('Unreadable'); } }), /Unreadable/);
});

test('checks again after the final bounded wait', async () => {
  let reads = 0;
  const result = await waitForFixtureUnmounts('/owned', {
    readMountInfo: async () => ++reads === 51 ? base : base + doc,
    pause: async () => {},
  });
  assert.deepEqual(result, { mountsObserved: 1, polls: 51, status: 'ready' });
});
