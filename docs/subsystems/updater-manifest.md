# Updater Manifest

## Responsibilities and sources of truth

[`assemble-updater.mjs`](../../scripts/assemble-updater.mjs) builds one `latest.json`
from the actual release asset filenames and their updater signatures. The
[`GUI Release` workflow](../../.github/workflows/gui-release.yml) runs it only
after every installer build has succeeded. This script does not sign assets or
prove that an installed application can update successfully.

Before building, [`configure-release.mjs`](../../scripts/configure-release.mjs)
sets the ephemeral Tauri configuration to the full tag version, including any
prerelease suffix. Source package versions remain at the base version. The
runtime updater compares Tauri's configured version with the manifest version;
leaving a candidate at the base version would make same-base prereleases appear
older and suppress their updates. The workflow restores the source configuration
after building.

## Runtime path

- Read downloaded assets from `updater-assets` and require exactly one bundle
  for each supported format and architecture.
- Bind each URL to that same bundle's nonempty `.sig` file.
- Keep each Linux architecture's generic fallback on its own AppImage, and publish
  explicit `linux-{x86_64,aarch64}-{appimage,deb}` entries. Tauri uses the
  format-specific entry first, so a DEB installation downloads DEB bytes.
- Keep Windows x86_64's generic entry on MSI and its explicit MSI/NSIS entries.
  Windows ARM64 has `windows-aarch64` and `windows-aarch64-nsis`, both bound to
  its native application's NSIS package. No ARM64 MSI entry is generated.
- Accept the native bundle filename aliases (`aarch64.AppImage`, `arm64.deb`,
  `arm64-setup.exe`) and both `x64.msi` and `x64_en-US.msi` forms. Reject missing,
  unknown, conflicting or unsupported architecture/format combinations. No
  missing ARM64 package can fall back to an x64 package.
- Write the complete manifest and checksums only after all bundles validate.
  Release filenames are URL-encoded, and the full release version is retained.

## Constraints and failure behavior

The runtime enables TLS and system-proxy support explicitly, with the updater's
legacy archive feature disabled. Windows receives signed EXE/MSI bytes and Linux
receives direct AppImage/DEB bytes; legacy Windows ZIP and Linux compressed
AppImage updates are unsupported. macOS application tarballs use the plugin's
native macOS extraction path and remain supported. This matches this repository's
manifest assembler and removes the ZIP dependency listed in
[RustSec's archive traversal advisories](https://rustsec.org/advisories/RUSTSEC-2026-0368.html).

A missing, duplicate, empty or unsigned DEB bundle must fail assembly rather
than let an installed DEB fall back to an AppImage. All other supported formats
retain the same validation across all six native targets. The release matrix
requires every target before its single manifest writer runs. Architecture
guards check the build host, application and Windows/Linux clock helper; the
NSIS installer wrapper itself is x86, while its application payload is ARM64.
An x64 application running under ARM emulation continues receiving x64 updates;
adding native ARM64 entries does not migrate that installation.
Fixture signatures test the manifest bindings only;
they are not cryptographic verification or actual update acceptance.

## Verification

```sh
node --test scripts/assemble-updater.test.mjs
node --test scripts/configure-release.test.mjs
```

The tests check all 15 platform keys, format-specific URLs and distinct bundle
signatures, retain x64 behavior, reject incomplete or ambiguous ARM64 assets and
unsupported ARM64 MSI, and verify checksums. Real old-version DEB and AppImage
update, restart and version readback still require signed release artifacts and
an isolated native desktop.
