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
| glib 0.18.5 | Backport the two-line upstream out-parameter fix | [Upstream PR 1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343), the checksum-verified [source and license record](../../app/src-tauri/vendor/README.md), optimized iterator old/new execution and native Linux verification |

The updater retains TLS and system-proxy support and this repository's current
signed formats. See [updater formats](updater-manifest.md). No signer key, updater
endpoint, tag, Release or published installer is changed by these source fixes.

## Remaining notices

- Linux's Tauri/GTK dependency chain retains the glib 0.18 API. RustSec reports
  [VariantStrIter unsoundness](https://rustsec.org/advisories/RUSTSEC-2024-0429.html),
  fixed in glib 0.20. A direct upgrade cannot satisfy GTK's 0.18 dependency range.
  The Cargo patch selects a complete local copy of the published 0.18.5 crate,
  with only the upstream fix's two lines changed. Its original version is retained;
  version-only scanners will still report this advisory. Source provenance and
  optimized native iterator evidence establish the backport separately. The
  [Linux regression](../../app/src-tauri/tests/glib_variant.rs) exercises all affected
  iterator methods; the test profile optimizes glib itself. Linux bundles include
  original license/copyright notices, independently compared after extraction in
  GUI CI. Source `947ef1ac` passed [native Linux test/package verification](https://github.com/ly87ing/agent-manager/actions/runs/39959737728),
  including the optimized iterator case and both extracted notice comparisons.
- zbus_names, zvariant, zvariant_derive, proc-macro-error and the resolved UNIC
  crates have unmaintained-package notices. These have no patched version in the
  queried advisories. Record replacement decisions separately from vulnerability
  fixes; source and native acceptance are required before a dependency migration.
- A clean frontend production audit does not cover frontend development tools,
  Rust dependencies, system libraries or installed binaries.

## Verification

The regular and GUI CI workflows prepare Ubuntu package sources with
[`ci-ubuntu-archive.mjs`](../../scripts/ci-ubuntu-archive.mjs) before APT runs.
Only exact Azure Ubuntu archive URI tokens in the two standard source files are
replaced with the official HTTPS archive. The `security.ubuntu.com` host, ARM ports,
third-party sources, suites, components and signing fields remain unchanged. The helper
refuses execution outside GitHub-hosted Linux CI and rereads changed files.
This transport change preserves [APT's existing signed archive verification](https://documentation.ubuntu.com/security/software-integrity/archive-verification/);
it does not add trust keys or disable authentication. Release workflows retain
their existing bootstrap and need separate acceptance before adopting this helper.

Check the pure rewrite and refusal cases with
`node --test scripts/ci-ubuntu-archive.test.mjs`. Native CI must also show actual
APT downloads from the selected archive; a rewritten file alone does not prove
download availability or speed.

Run the existing Tauri tests, frontend checks and release-script tests. Compare
the actual resolved graph before and after changes:

```sh
cargo tree --manifest-path app/src-tauri/Cargo.toml --target all -e normal,build
cargo test --manifest-path app/src-tauri/Cargo.toml --test glib_variant
node --test scripts/assemble-updater.test.mjs scripts/tauri-plugin-versions.test.mjs
pnpm --dir app audit --prod --registry=https://registry.npmjs.org
```

Re-query advisories for the actual locked package/version pairs and distinguish
aliases from separate findings. Native Windows/Linux builds and signed installed
update verification remain separate requirements; a resolved graph is not runtime
acceptance. Never run an update against a user's active installation as a test.

The iterator case executes only on native Linux; zero cases on macOS/Windows do
not establish it. Confirm `cargo tree` resolves glib to the recorded local path.
For backport review, unpack the published archive after verifying its recorded
SHA256 and compare all files: only the pointer binding and out-reference in
`src/variant_iter.rs` may differ.
