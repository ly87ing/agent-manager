# Updater Manifest

## Responsibilities and sources of truth

[`assemble-updater.mjs`](../../scripts/assemble-updater.mjs) builds one `latest.json`
from the actual release asset filenames and their updater signatures. The
[`GUI Release` workflow](../../.github/workflows/gui-release.yml) runs it only
after every installer build has succeeded. This script does not sign assets or
prove that an installed application can update successfully.

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

A missing, duplicate, empty or unsigned DEB bundle must fail assembly rather
than let an installed DEB fall back to an AppImage. All other supported formats
retain the same validation. Fixture signatures test the manifest bindings only;
they are not cryptographic verification or actual update acceptance.

## Verification

```sh
node --test scripts/assemble-updater.test.mjs
```

The tests check format-specific URLs and distinct bundle signatures, reject
incomplete DEB assets, and verify checksums. Real old-version DEB and AppImage
update, restart and version readback still require signed release artifacts and
an isolated native desktop.
