# Extension and Tool Backups

## Responsibilities and sources of truth

[`native_config.rs`](../../engine-rs/src/native_config.rs) exports portable extension
definitions and restores them in one SQLite transaction. The Settings interface
previews differences in the selected modules before confirmation.

Backups include skill sources, saved inactive MCP items, rule templates, saved
launch recipes and environment expected-value collections. They
exclude account/alias registrations, credentials and secret-file paths, active
assistant configuration, assignments, ownership digests, and operation history.
Restore never installs programs, applies resources to assistants, launches an
application, queries a time server or changes system settings.

## Runtime path and failure behavior

`config:export` is read-only and returns a `base` digest over the exported
definition state, excluding its timestamp. `config:import --value JSON --modules
skills,mcp,rules,launch,environment --base DIGEST` restores only selected modules.
Skills, rule templates, launch recipes and environment collections replace their
module contents; saved MCP items merge by identity,
preserving local items absent from the backup. Legacy rule text converts to
templates only when rules is selected. Omitted legacy template sections leave
existing templates intact. CLI imports without `--modules` retain the historical
whole-definition import contract. The additive `launch_recipes` and
`environment_profiles` sections keep format v1. Missing or null sections in an
older backup preserve local tool data; an explicitly empty selected section
clears that module. Tool IDs are preserved, environment names remain unique,
and the normal recipe/profile validators and secret rejection run before writes.
Restored automatic-correction flags never trigger correction. Comparisons remain
read-only; system changes require a separate apply or repair confirmation.

Unknown or empty module selections fail. Validation still rejects credentials
and changes to immutable account/alias metadata, even in an unselected module.
After acquiring the database write transaction, restore re-exports current state
and rejects a stale preview baseline before any modification. A transaction
failure leaves all definitions intact. Existing actual assistant files and other
modules remain unchanged. No automatic backup files are created; users can
explicitly export the current definitions before restoring.

The baseline includes both tool sections, so a concurrent recipe or collection
edit invalidates the preview too. Restoring tools and writing the operation audit
share the existing transaction; an audit failure rolls back both tool tables.

Restore takes the same exclusive apply lock as skill writes. In its database
transaction it refuses removing or replacing a source that still has assigned
skills, including disabled assignments. Remove those assignments and preview
again before restoring the changed source. Unselected skill definitions remain
untouched. This prevents definitions from disappearing underneath active links.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_config
cd app && pnpm vitest run src/test/ConfigBackup.test.tsx src/test/backupDiff.test.ts
```

The isolated integration case exercises each selected module, unknown/empty
selections and a rule edit after preview, then reads back the definitions and
checks that no main assistant directory was created.
