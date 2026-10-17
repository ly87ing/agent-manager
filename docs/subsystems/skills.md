# Skill Assignment and Source Switching

## Responsibilities and sources of truth

[`native_skills`](../../engine-rs/src/native_skills/mod.rs) discovers skills and
reads actual assistant links. SQLite stores assignments and content versions;
account registrations are immutable metadata. Local sources remain user-owned;
Git sources provide verified content snapshots.

New Git additions, explicit updates and unassigned Git forks use typed v2
SHA-256 IDs. The hash starts with `\0agent-manager-skill-v2\0` and includes sorted
file, link and directory entries: kind, lossless UTF-8 relative component path,
content or raw-target digest, and the link's normalized root-relative target,
separated by NUL bytes. Included empty directories affect v2; excluded directory
names retain their existing behavior. Included unsupported entry types, reparse
points other than supported symbolic links, and non-UTF-8 paths or targets cannot
produce a new typed version.
Typed link inspection reads the OS target before any comparison helper strips
Windows namespace prefixes; absolute/drive/namespace targets are refused.

Git sources also interpret ordinary files declared as exact stage-zero `120000`
entries in the cache's index. This supports Git's `core.symlinks=false` checkout:
current file bytes supply the raw target, rather than guessing a link from text
or reading an old blob. Replacing that representation with an OS link having
the same raw target preserves the logical typed version. The legacy fingerprint
continues to describe physical entry kinds; immutable snapshot verification never
reads a Git index.

The in-process [index reader](../../engine-rs/src/native_skills/git_index.rs)
accepts full v2/v3 indexes up to 4 MiB. It validates unique sorted stage-zero
paths, modes, flags, lengths, padding, file/directory conflicts and a unique
SHA-1/SHA-256 layout. Nonzero checksums must match; Git's documented zero footer
is accepted. Checksums and index object IDs do not authenticate content, HEAD or
the repository's configured object format. Optional extensions are skipped by
checked length, without recursive decoding. Missing, malformed, v4, split,
sparse, unresolved or unknown mandatory layouts remain unknown. Linked `.git`
files, reparse metadata roots and `commondir` layouts are refused; queries do not
repair or normalize them.

[Source interpretation](../../engine-rs/src/native_skills/git_source.rs) checks
native directory-entry spelling for every indexed link and gitlink, including
other prefixes, before matching a selected skill. A system-resolved alias with
different spelling is refused without inventing Unicode case-folding rules.
Directory-name inspection shares a 4 MiB budget, including cached path/name
storage, and 30-second checkpoints per source construction or selected-root walk.
These are observed filesystem checks, not atomic traversal or a hard deadline
for blocking filesystem calls or the whole status operation. Source contents
still require their normal full-file fingerprints. Selected roots must be
ordinary directories inside the known repository; retained identities are checked
before and after inventory. Included subtrees overlapping any gitlink are unknown.
Missing indexed links, including changed spelling, require explicit correction
of the source/index before new versions can be created.

Status parses each cached source index once, including comparisons for personal
fork provenance. Interpretation errors appear in `source_version_error` and the
collection warning; new assignment/content-update actions are withheld. A safely
scoped, unchanged legacy comparison can remain known. Existing verified versions
can still be enabled or disabled when the skill remains discoverable. A missing
cache is a separate discovery boundary. New Git additions, updates and unassigned
forks never silently deploy an uninterpreted target-text file.

Each typed version has an immutable `snapshot.json` outside `content`, declaring
schema 2 and its version ID, or the Windows materialization described below.
Schema 2 metadata remains limited to 4096 bytes. Verification reads the actual contents.
A present malformed, oversized,
unknown-schema, duplicate-field or directory/reparse manifest is refused. Only
an absent manifest selects the original hash format. Removing a typed manifest
or adding one to a legacy snapshot does not preserve a valid version.

On Windows, new Git snapshots whose links all point directly to included ordinary
files use schema 3 and ordinary file copies. Plaintext Git links use this writer
on every platform. Their target text must be nonempty UTF-8 without NUL and within
64 KiB; unsupported plaintext directory/chained targets are refused before staging,
unless the exact version already exists and passes its recorded verifier.
The dedicated writer copies each
validated source target, even if its name sorts after the alias; it never attempts
to create a file symbolic link. Eligible directory/mixed-link sources use schema 4
below. Other actual OS-link sources retain schema 2 and its native capability
requirements, including refusal when the copied link kind/meaning cannot preserve
the fixed version. Plaintext checkouts never use that native-link fallback.
Existing versions are verified with their recorded format before new writer
eligibility is considered; stricter creation rules do not reinterpret them.

Schema 3 declares exactly its schema, version and `materialized_file_links`, each
containing only `path` and `raw_target`, within 64 KiB. Readers on every platform
inventory all physical files and directories without source exclusions, refuse
links/reparse entries, and require unique alias records. Windows relative-path
rules apply on every verifier host: exact spelling, no drive/namespace/device/ADS
names, and no traversal through missing entries, files or aliases. Each alias's
actual default-stream digest must match its included ordinary non-alias target.
Replacing only those file entries with their recorded logical links must produce
the caller's expected typed v2 ID. Receipt text cannot override a fixed ID;
additional files or empty excluded directories invalidate it.

Schema 4 adds required `materialized_directory_links` alongside the required file
array; directory records must be nonempty, file records may be empty. Both contain
only `path` and `raw_target`. The serialized receipt must fit 64 KiB before staging.
Both arrays share unique, nonoverlapping alias paths and the same strict traversal
policy. Directory targets must resolve to an included ordinary sibling directory,
with no links anywhere in its included subtree. Creation copies only that logical
inventory, including empty directories; source exclusions still apply to ordinary
source directories. File-only snapshots continue to use schema 3.

The schema 4 verifier inventories all physical entries, compares each directory
alias's exact relative paths, kinds and file digests with its target, then removes
only proved virtual descendants and restores the recorded logical links. The
result must match the original fixed typed v2 ID. Matching changes to both copies
still fail. ASCII case-colliding names are refused. Non-ASCII names with case
transformations remain unavailable for this materialization rather than assuming
Rust's Unicode mapping equals Windows filesystem mapping; uncased names such as
Chinese remain eligible. This policy does not reinterpret schema 2/3.
It is not a complete Unicode filesystem model: normalization equivalence and
component-length limits still depend on the destination filesystem. Failure to
create or verify the exact namespace refuses publication; it never merges names.

Directory materialization promises read-only default-stream subtree contents.
It does not preserve `readlink`, `realpath`, physical working-directory identity,
inode/hardlink identity or write coupling between copies. Non-sibling targets,
alias traversal, chained or cyclic aliases are outside this new representation.
On Unix, actual OS-link sources continue to use their native schema 2 path.

Each schema 4 expansion is limited to 65,536 nodes, 4 MiB of combined output/input
path accounting (and doubled physical-key accounting), and 512 MiB of total
default-stream bytes. Planning checks lengths through ordinary handles; actual
copy and physical hashing consume fresh budgets, including growing files. Before
staging, filesystem availability at the planned `skill-versions` parent must cover
the payload plus 64 MiB headroom. This is advisory, without space reservation;
copy failures retain the existing owned-stage cleanup and no-publication rules.
These limits do not bound the earlier source fingerprint, old formats or the
whole process footprint. See [Resource bounds](resource-bounds.md).

The new writer creates files exclusively and copies default-stream bytes without
propagating NTFS alternate streams. On Unix, new copied files use the existing
snapshot policy of mode `0444`; personal forks make their own copies writable.
Typed identities cover names, kinds, included directories, raw link targets and
default-stream digests; ACLs, timestamps,
hardlink identity and alternate streams are outside that existing content model.
A personal fork of a materialized snapshot contains editable ordinary files and
does not inherit the sibling integrity manifest. Schema 2-only readers cannot
manage schema 3 bindings: rollback must retain a capable verifier or restore a
capable build, never remove manifests or rewrite IDs/bindings. Schema 4 likewise
requires a capable reader. Its offline personal fork copies the complete verified
ordinary namespace, including directory aliases named `node_modules`, instead of
reapplying source exclusions. The copy is independently verified before
registration. Receipt removal or format changes before fork require the actually
observed format's fixed-ID proof, never a field-only fallback to generic copying.

Existing snapshots and bindings retain their original IDs and are not rewritten.
Local assignments still follow their actual sources with the legacy hash. Status
computes both source fingerprints in one normal inventory and compares each
assistant with its recorded format, so unchanged mixed v1/v2 bindings do not show
false updates. If typed validation fails, a provably unchanged legacy comparison
remains known; the current typed comparison is unknown. An explicit Git update
selects v2. Legacy verification keeps its original limitations, including the
untyped file/link digest ambiguity; it is not silently repaired by compatibility.

Before publishing a new snapshot, the copied staging contents must match the
exact version chosen for the assignment. Copy errors or mismatches remove only
that operation's uniquely created staging directory and leave existing versions
and bindings unchanged. Reusing or enabling a version retains strict content
verification; an unrecorded ordinary-file replacement for a symbolic link is
refused. Only a verified schema 3 record permits that representation.
An existing version entry is always verified; an incomplete directory, directory
link or reparse root is refused. Publication uses the platform's no-replace
directory rename, including when an empty destination appears after preparation.
Staging cleanup and publication retain open parent, staging and content root
handles and recheck their recorded file identities, including after publication.
Changed or uncertain ownership stops cleanup and preserves the entry with an
error. Content roots must be ordinary directories; reuse holds their original
handles while hashing and rechecking identities. These checks detect observed
replacement; they do not lock every source path or every content byte against an
actively concurrent writer.
They do not provide atomic traversal against concurrent child-directory replacement.
On Windows, retained references open directories by their full file IDs after
checking ordinary pathname roots, then independently recheck identity and type.
This avoids pathname-bound descendants blocking ancestor rename while preserving
the original objects. Filesystems that cannot provide those references are
refused; there is no numeric-ID-only fallback or permission relaxation.

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
On Windows, copied ordinary files have their read-only attribute cleared;
links and other reparse points are never followed to change target attributes.
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
cargo test --manifest-path engine-rs/Cargo.toml --lib native_skills::snapshot_v2
cargo test --manifest-path engine-rs/Cargo.toml --lib native_skills::snapshot_materialized
cargo test --manifest-path engine-rs/Cargo.toml --lib native_skills::snapshot_directory
cargo test --manifest-path engine-rs/Cargo.toml --lib native_skills::snapshot_budget
cargo test --manifest-path engine-rs/Cargo.toml --lib native_skills::git_index
cargo test --manifest-path engine-rs/Cargo.toml --test native_skill_git_links
cargo test --manifest-path engine-rs/Cargo.toml --test native_skills typed
cargo test --manifest-path engine-rs/Cargo.toml --test native_skills_windows
cd app && pnpm vitest run src/test/SkillsPage.test.tsx src/test/SourcesLifecycle.test.tsx src/test/rollback.test.ts
```

The integration case reads actual links and assignments after stale-version,
missing-source, external-link and injected commit failures, then verifies a
successful switch and the unchanged second assistant and account definitions.
Typed snapshot checks cover fixed fingerprints, the legacy file/link ambiguity,
raw target spelling, empty directories, manifest corruption/deletion, mixed
bindings, zero-write status, enable/disable, explicit update, offline bound fork
and unassigned Git fork. Unix path-key tests reject invalid UTF-8 without needing
the host filesystem to create such names; the real invalid-filename fixture runs
on Linux. Windows link tests require the native CI runner's existing symlink
capability to prepare source fixtures. Materialization tests independently read
ordinary, non-reparse output files, later-sorted targets, nested traversal,
verbatim Windows roots, absent copied alternate streams, immutable snapshot reuse,
disable/enable, tamper refusal and an offline personal fork through the native CLI.
The Windows-only CLI case must execute on a native Windows runner; its zero-test
result on another host does not establish Windows acceptance. Cross-host verifier
cases reject forged receipts, duplicate/unknown fields, extra excluded directories,
physical links, ambiguous Windows paths and alias/target changes.
The cross-platform Git-link CLI cases prepare real `core.symlinks=false`
checkouts in SHA-1 and SHA-256 repositories, then remove Git from the CLI's PATH.
They read back payloads, receipts, assignment isolation, unknown-index behavior
and Git metadata bytes/timestamps. Case-insensitive filesystem alias challenges
report whether the host actually exercised them; a case-sensitive host does not
establish that acceptance. Directory-root aliases, nested gitlinks and unsupported
plaintext directory links must not publish target-text snapshots.
