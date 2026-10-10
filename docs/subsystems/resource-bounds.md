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
