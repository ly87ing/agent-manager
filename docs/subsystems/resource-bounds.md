# Resource bounds

## Responsibilities and sources of truth

Skill fingerprints in [snapshot.rs](../../engine-rs/src/native_skills/snapshot.rs)
identify source content, verify immutable snapshots, and protect adoption and
rollback. Actual files remain authoritative.

## Runtime path

`hash_skill_dir` reads file contents through one reused 64 KiB buffer. It hashes
every byte and retains the existing sorted path/digest format. Symlink handling,
excluded directories, and final fingerprint ordering are unchanged.

## Constraints and failure behavior

Unreadable files and directories fail; an interrupted read retries. Large file
contents no longer require an allocation proportional to file size. Directory
paths and fingerprint entries still grow with the number of files; this is not a
fixed bound on the whole process or desktop footprint.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --lib native_skills::snapshot::tests
cargo test --manifest-path engine-rs/Cargo.toml --test native_skills
```

Use the same large local source and isolated HOME/data to compare CLI peak RSS and
fingerprints against the previous build. See the [goal record](../goal-lightweight.md)
for the measured workload and results.

## Frontend retention and query work

The execution provider retains the newest 50 completed operations and all running
operations. It keeps at most 50 resource-step summaries and 20 diagnostics, each
text field limited to 2,048 characters. Full command results are returned to the
caller and fetched from the audit store only when a user opens details.
Inactive query cache entries expire after 60 seconds. Known resource writes
invalidate their domain and its cross-domain dependencies; unknown writers retain
the conservative all-domain refresh. Batch resource writes refresh once in finally,
including partial failures. Runs polls only while an operation is running, and
progress polls cannot overlap.

The session retains the newest 50 completed batch summaries and all running
batches. Summaries keep target identities, status and at most 2,048 characters
per error, without command payloads or credentials. Skills and Sources show the
latest result; Runs lists grouped results alongside individual audit records.
Retry selects failed rows only, re-reads `skills:status` before each target and
requires the action to remain available. External takeover is not replayed.
Completed groups do not persist across application restarts; individual audit
records do. A partial source application must not report full success.

Source updates read fresh skill status before deployment and report completed
row counts only after the batch returns. Failed source fetches are excluded from
deployment; failed status reads and refused batches report an incomplete result.
Sources shows query and envelope failures instead of an empty inventory, and
disables editing until its source, rule and definition queries succeed.
MCP writes invalidate program dependency status as well as MCP inventory.

Account and controlled-launch operations are retired. Extension selectors and
Assistants use `resources:targets`: immutable metadata only, without loading
secrets or invoking assistant executables. Sources uses
`skills:status --collections-only`, discovering names and source metadata without
building skill fingerprints. Per-skill health is deferred; update/copy actions
fetch full status before planning writes. Dependency status runs only when its owning page is mounted and
performs no executable version probes or network access.

## Process output and previews

MCP checks retain at most 1 MiB per stdout line, 4 MiB total, and 16 queued lines.
Oversized or flooded output fails explicitly. Stderr keeps its existing bounded
tail; HTTP body accumulation uses one string rather than a per-line vector. Normal
process-group cleanup, credential scrubbing, and account locks remain in place.
Git previews retain at most 64 KiB plus one sentinel byte per blob through the
shared deadline-aware process capture. Oversized files remain distinguishable from
empty and missing files; previews still display at most 30 file contents.
These limits bound payload retention, not the number of configuration items or the
entire WebKit/application memory footprint.

Additional checks:

```sh
cargo test --manifest-path engine-rs/Cargo.toml --lib native_mcp::check::tests
cargo test --manifest-path engine-rs/Cargo.toml --lib git_preview_retains_a_bounded_blob
cargo test --manifest-path engine-rs/Cargo.toml --test native_core metadata_status
cargo test --manifest-path engine-rs/Cargo.toml --test native_skills collection_view
cd app
pnpm test:unit run src/test/executionBounds.test.tsx src/test/ProgramsPage.test.tsx src/test/SourcesPage.test.tsx src/test/RunsPage.test.tsx
```
