# Interface Language

## Responsibilities and sources of truth

The desktop interface defaults to English, regardless of OS or browser language.
Settings → Language offers English and 简体中文. The existing `native-manager.db`
`app_settings` table stores `language` as `"en"` or `"zh-CN"`; there is no separate
language configuration file. Missing preferences, including legacy databases,
mean English. Reading defaults does not create or update the database.

[i18n.ts](../../app/src/lib/i18n.ts) holds the current presentation language;
[messages.ts](../../app/src/lib/messages.ts) contains the English catalogue,
keyed by the existing Chinese copy. Only application-owned text is translated.
Resource names, paths, rule/skill contents, interpolated user values and original
engine diagnostics retain their original text.

## Runtime path

1. [Settings](../../app/src/lib/settings.ts) reads the native preferences.
   The active query snapshot updates the language before the interface paints.
2. Selecting a language edits a draft. **Save settings** sends language and log
   retention together to [the native settings handler](../../app/src-tauri/src/settings.rs).
3. The handler reads current preferences inside the existing SQLite transaction,
   preserves the secrets-file path, and writes app preferences only.
4. Successful readback updates the language and query cache. Navigation, page
   labels, forms, search labels and date formatting refresh without a restart.
   Reopening the desktop app reads the saved preference.

## Constraints and failure behavior

- Unsupported language values are rejected by Rust and the frontend schema.
- A failed save keeps the active language and leaves the draft available to retry.
- A canceled older query cannot replace the newly saved language. Fetching data
  has no global language side effects; only the active snapshot applies it.
- Display metadata resolves translations at render time. User content is never
  searched or rewritten to match the catalogue.
- Localized MCP “keep existing” hints serialize to the unchanged `<literal>`
  engine sentinel, including when the form's language changes.
- Language settings do not edit assistant configuration, identity, sessions or
  target registrations. All extension-only boundaries remain in force.
- Older binaries reject unknown preference rows. Before downgrading to a binary
  without language support, back up the application database and remove only its
  `app_settings` row with key `language`; do not remove the database or assistant files.

## Verification

```sh
pnpm --dir app test:unit run src/test/Language.test.tsx src/test/McpPage.test.tsx
cargo test --manifest-path app/src-tauri/Cargo.toml settings::tests
cargo test --manifest-path engine-rs/Cargo.toml store::tests::settings
pnpm --dir app build
```

Check every page at approximately 1000 and 760 px in both languages. In an
isolated HOME/data directory, save Chinese through the native UI, read back the
database, restart and verify Chinese, then save English and verify the same
round trip. Compare simulated assistant configuration and identity files before
and after. Browser mock state proves presentation only, not desktop persistence.
