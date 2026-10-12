# Skill Assignment and Source Switching

## Responsibilities and sources of truth

[`native_skills`](../../engine-rs/src/native_skills/mod.rs) discovers skills and
reads actual assistant links. SQLite stores assignments and content versions;
account registrations are immutable metadata. Local sources remain user-owned;
Git sources provide verified content snapshots.

## Runtime path and failure behavior

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

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_skills replacing_a_skill
cd app && pnpm vitest run src/test/SkillsPage.test.tsx src/test/rollback.test.ts
```

The integration case reads actual links and assignments after stale-version,
missing-source, external-link and injected commit failures, then verifies a
successful switch and the unchanged second assistant and account definitions.
