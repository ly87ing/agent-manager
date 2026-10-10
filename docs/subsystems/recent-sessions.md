# Retired recent sessions

## Responsibilities and sources of truth

Recent sessions and resume shortcuts have been retired by the
[lightweight goal](../goal-lightweight.md). The desktop has no Usage page.
The [native dispatcher](../../engine-rs/src/native.rs) rejects usage:sessions,
along with usage:status, usage:scan, and usage:clear, before reading session files
or opening the database. Assistant sessions remain the assistant's own data.

## Constraints and failure behavior

Retired commands return FEATURE_RETIRED. They do not create application data,
scan conversations, or delete existing database rows or session files.
The retired collector implementation is removed; retirement tests remain.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test usage --test agent_usage
```

Fixtures exercise both fresh directories and preserved legacy data. Actual session
resume behavior is outside this tool's scope.
