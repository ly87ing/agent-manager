# Dependency Security

Dependency checks cover the resolved versions in both Cargo lockfiles and the
frontend production dependency graph. They do not prove installed application
behavior or that every advisory is reachable. Keep vulnerabilities, unsoundness
notices and unmaintained-package notices separate; do not silently suppress them.

## 2026-10-23 review

The review started at `156849bc`, queried OSV for locked crates and checked the
following findings against RustSec and the actual updater dependency/call paths.

| Dependency | Correction | Evidence |
| --- | --- | --- |
| rustls 0.23.43 | Resolve 0.23.46 | Patched TLS handshake, session-debug, decompression and key-share advisories: [0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html), [0346](https://rustsec.org/advisories/RUSTSEC-2026-0346.html), [0348](https://rustsec.org/advisories/RUSTSEC-2026-0348.html), [0349](https://rustsec.org/advisories/RUSTSEC-2026-0349.html) |
| rustls-webpki 0.103.15 | Resolve 0.103.16 | Reject distrusted intermediate keys: [0382](https://rustsec.org/advisories/RUSTSEC-2026-0382.html) |
| webpki-root-certs 1.0.9 | Resolve 1.0.10 | Remove the affected bundled root: [0383](https://rustsec.org/advisories/RUSTSEC-2026-0383.html) |
| zip 4.6.1 | Remove the optional updater archive feature and dependency | The Windows updater called `ZipArchive::extract`; remove the dependency rather than relying on platform-specific reachability of [0367](https://rustsec.org/advisories/RUSTSEC-2026-0367.html), [0368](https://rustsec.org/advisories/RUSTSEC-2026-0368.html), [0369](https://rustsec.org/advisories/RUSTSEC-2026-0369.html) |

The updater retains TLS and system-proxy support and this repository's current
signed formats. See [updater formats](updater-manifest.md). No signer key, updater
endpoint, tag, Release or published installer is changed by these source fixes.

## Remaining notices

- Linux's Tauri/GTK dependency chain still resolves glib 0.18.5. RustSec reports
  [VariantStrIter unsoundness](https://rustsec.org/advisories/RUSTSEC-2024-0429.html),
  fixed in glib 0.20. A direct upgrade cannot satisfy GTK's 0.18 dependency range.
  The reviewed application, Tauri, Wry, GTK, GIO, GDK and WebKit source paths do not
  call `array_iter_str`; that search is not proof of absence in all runtime paths.
  Keep Linux production acceptance open pending an upstream-compatible fix or a
  reviewed backport with native verification.
- zbus_names, zvariant, zvariant_derive, proc-macro-error and the resolved UNIC
  crates have unmaintained-package notices. These have no patched version in the
  queried advisories. Record replacement decisions separately from vulnerability
  fixes; source and native acceptance are required before a dependency migration.
- A clean frontend production audit does not cover frontend development tools,
  Rust dependencies, system libraries or installed binaries.

## Verification

Run the existing Tauri tests, frontend checks and release-script tests. Compare
the actual resolved graph before and after changes:

```sh
cargo tree --manifest-path app/src-tauri/Cargo.toml --target all -e normal,build
node --test scripts/assemble-updater.test.mjs scripts/tauri-plugin-versions.test.mjs
pnpm --dir app audit --prod --registry=https://registry.npmjs.org
```

Re-query advisories for the actual locked package/version pairs and distinguish
aliases from separate findings. Native Windows/Linux builds and signed installed
update verification remain separate requirements; a resolved graph is not runtime
acceptance. Never run an update against a user's active installation as a test.
