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
- Keep the generic Linux x86_64 fallback on AppImage, and publish explicit
  `linux-x86_64-appimage` and `linux-x86_64-deb` entries. Tauri uses the
  format-specific entry first, so a DEB installation downloads DEB bytes.
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
retain the same validation. Fixture signatures test the manifest bindings only;
they are not cryptographic verification or actual update acceptance.

## Verification

```sh
node --test scripts/assemble-updater.test.mjs
node --test scripts/configure-release.test.mjs
```

The tests check format-specific URLs and distinct bundle signatures, reject
incomplete DEB assets, and verify checksums. Real old-version DEB and AppImage
update, restart and version readback still require signed release artifacts and
an isolated native desktop.
