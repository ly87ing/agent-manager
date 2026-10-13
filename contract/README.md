# Engine Contract v1

The desktop and CLI call the same Rust dispatcher. Responses use the envelope in [envelope.schema.json](engine-v1/envelope.schema.json); status elements use [status.schema.json](engine-v1/status.schema.json). Assistant files and directories are authoritative for active extensions. SQLite holds preferences, source definitions, ownership records, operation history, saved inactive alternatives, and general launch recipes.

## Current command surface

- `resources:targets` lists existing extension targets from immutable registration metadata. It does not inspect authentication, account settings, or assistant executables.
- `mcp:status/preview/check/apply` reads and changes one MCP entry for one assistant. The status response includes each target file and a per-entry digest. Preview is read-only. Apply compares the digest under the write lock, preserves unrelated fields, and conditionally rolls back its own writes.
- `skills:status/preview/apply/fork/versions/adopt-preview/adopt` manages skill sources, links, assignments, and versions. `system:discover --only skills` classifies existing local skills without writing.
- `rules:status/apply/save-template/delete-template` reads actual rule files, applies one selected source or value with a file baseline, and stores inactive templates. Recommended rule text comes from skill sources.
- `programs:status/preview/apply/declare` manages programs required by MCP or skills; assistant runtimes are excluded.
- `secrets:status/set/source` handles dedicated MCP variables in a selected external private file. Assistant identity variables and historical credential ownership remain protected.
- `launch:catalog/build/list/save/delete/run` builds general commands, saves inactive recipes, and can explicitly start a local graphical application. It does not resolve account identity or write shell configuration.
- `policy:get/set/delete`, `config:export/import`, `runs:list/get/clear/progress`, `system:audit/refresh-upstream`, and `db:status/migrate` support the active extension workflow. Policy writes use `--layer local`; backup import cannot change historical account or alias metadata.

Account management, authentication, assistant installation, provider/model/runtime editing, account-bound launch, historical shell-alias writers, hooks, and usage collection are retired. Account, alias, and usage domains return `FEATURE_RETIRED` at the engine boundary; removed domains such as hooks are rejected as unknown. The Tauri whitelist rejects all of them. The old `--account-launch` entry point immediately refuses execution. Existing assistant files, sessions, scripts, registrations, and database rows are preserved.

Queries and previews create no database or assistant file. Active writes reread actual content, validate the selected target, compare the displayed baseline, and refuse external changes. Saving a source or inactive alternative does not apply it. See [Extensions only](../docs/EXTENSIONS-ONLY.md), [data semantics](../docs/GUI-REDESIGN.md#data-model-the-actual-configuration-is-authoritative), and [extension safety](../docs/subsystems/account-context.md).

## Schemas and verification

`engine-v1/` contains the envelope, element, status, apply-result, policy, and runs schemas. The TypeScript response model is [contract.ts](../app/src/lib/contract.ts).

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
pnpm --dir app test:unit run
pnpm --dir app typecheck
pnpm --dir app build
```
