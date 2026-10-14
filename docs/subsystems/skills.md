# Skill Assignment and Source Switching

## Responsibilities and sources of truth

[`native_skills`](../../engine-rs/src/native_skills/mod.rs) discovers skills and
reads actual assistant links. SQLite stores assignments and content versions;
account registrations are immutable metadata. Local sources remain user-owned;
Git sources provide verified content snapshots.

## Runtime path and failure behavior

Personal collections default to `~/skills/<collection>/<skill>/SKILL.md`.
`skills:fork --collection NAME` derives the collection directory from its name;
`--dest PATH` selects a custom collection directory. An existing local collection
always reuses its registered `source_path`, including historical `~/my-skills`
locations. The interface shows the complete destination before confirmation.

Git repository caches remain grouped by collection under the application data
directory. Existing sources and assistant entries are never moved automatically.

For unmanaged skills, `skills:adopt-preview --name NAME [--dest PATH] [--local-only]` previews
the personal `my-skills` collection. The local digest includes its resolved
directory and existing definition. `skills:adopt --keep-local` must use that
same preview and directory; a changed directory or definition requires a new
preview before any content is copied or moved. Editing the destination in the
interface disables confirmation until the preview is refreshed and retains the
selected assistants. Source adoption
and duplicate cleanup do not depend on a usable personal-copy destination.

The desktop initially uses `--local-only`: it reads local provenance and prepares
the personal-copy plan without querying Git. **Compare remote source** explicitly
requests remote refs and content comparison. A remote failure returns the local
option; changing the personal destination refreshes only the local preview.
CLI callers omit `--local-only` when they want the existing remote comparison.

Personal-copy destinations reject account and application-data directories,
assistant skill roots, directory aliases, ambiguous paths, and overlap with other
local collections. Comparisons resolve existing ancestors even when the final
directory does not exist. Copies never overwrite an existing skill directory.

`skills:fork` shares the exclusive apply lock with assignment writes and backup
restore. It creates the skill directory exclusively and revalidates its destination
against the current policy inside the registration transaction. Failed copying
or registration removes the new copy only while its cleanup path remains safe.

Duplicate skill names within the selected search paths block discovery and
deployment with a `duplicate-name` diagnostic containing the conflicting paths.
Existing assignments remain removable. Historical assignments without a source
definition stay visible with removal-only actions; unknown local links require
individual takeover confirmation. Real directories and another source's
same-name assignments remain protected.

Source definition edits and deletions share this lock and cannot remove or change
the location of an assigned collection. Tracking and selection metadata can still
be edited without changing the registered source location.

Collection folders organize the source library. Assistant entries remain
`<assistant-skills>/<skill>` links: nested deployment is not enabled without
actual discovery acceptance for the affected assistant. Collection directories
do not namespace skill names or bypass same-name ownership checks.

Creating a personal version first copies and registers the skill with `skills:fork`.
Git assignments copy their verified content snapshot. Local assignments copy the
current source directory because their active links follow that source directly;
they do not have a content snapshot. Editing the copy never changes the source.
Each selected assistant then uses `skills:apply --action add --only skill:NEW/NAME
--agent TARGET --replace-from skill:OLD/NAME --replace-version VERSION`.
The old assigned version must still match, and an existing link must point to that
exact source. Real files and external or other managed links are refused.

Under the normal exclusive apply lock, the engine verifies the new source before
replacing the link. Old and new assignments are updated in one database transaction.
A commit failure restores the previous link only if the link still matches this
operation's replacement; external changes are never overwritten by rollback.
Missing original links may be repaired while retaining the original binding until
success. Other assistants remain untouched. A failed switch leaves the personal
copy available for review and a later retry.

Ordinary add rollback must not be offered for source replacement: removing the new
assignment alone cannot restore the previous source.

Collection summaries wrap names, source paths and status marks inside their own
column while assistant counts keep fixed aligned columns. Source locations remain
fully readable at narrow widths.

An unassigned same-name row whose actual link matches another active assignment
is `other-source`, with no takeover or removal action. Ordinary add, including
external takeover, refuses another active assignment; changing sources requires
the explicit original collection/version replacement contract above.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_skills replacing_a_skill
cargo test --manifest-path engine-rs/Cargo.toml --test native_skills personal_skill
cd app && pnpm vitest run src/test/SkillsPage.test.tsx src/test/SourcesLifecycle.test.tsx src/test/rollback.test.ts
```

The integration case reads actual links and assignments after stale-version,
missing-source, external-link and injected commit failures, then verifies a
successful switch and the unchanged second assistant and account definitions.
