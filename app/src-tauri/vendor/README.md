# GLib 0.18 security backport

`glib/` is the complete published glib 0.18.5 crate with the two-line upstream
[VariantStrIter fix](https://github.com/gtk-rs/gtk-rs-core/pull/1343) backported.
It changes the out-parameter from `&p` to `&mut p`, with a mutable pointer binding.
All other files match the source archive; original license and copyright remain.
Linux bundles carry both notices under `licenses/glib/`; GUI CI compares the
extracted AppImage and DEB notices with these originals.

- Source: <https://static.crates.io/crates/glib/glib-0.18.5.crate>
- Archive SHA256: `233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5`
- Upstream 0.18.5 source: `42b9caf98e03ded086362d9653ca58fe94dc8658`
- Advisory: [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html)

GTK's current 0.18 dependency range prevents a direct move to the fixed 0.20 API.
The Cargo patch keeps the original version and dependency interfaces. Version-only
scanners may still report the advisory; do not relabel the crate or suppress that
fact. Compare source, the selected Cargo path and optimized native test evidence.
Remove this patch after the desktop dependency chain accepts a maintained fixed
version, with the same native iterator and application checks.

`tests/glib_variant.rs` exercises the affected forward/backward iterator methods
and mixed iteration, including empty and Unicode values. The test profile optimizes
glib itself so normal native Linux Tauri tests exercise the defect's trigger.
