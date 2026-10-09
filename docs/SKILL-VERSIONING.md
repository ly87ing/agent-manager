# Skill Sources: Versioning and Adoption

Status: **design confirmed and fully implemented (2026-09-29)**. Phases A, B, and C and phase-B preview are complete; implementation and acceptance are in the [task document](goal-rust-gui.md). This refines Skills in section 3.2 of the [multi-account design](MULTI-ACCOUNT-DESIGN.md) and takes precedence if they conflict.

## Goal

Every skill has a clear source and tracking method. Manager checks for new versions according to tracking, updates or rolls back after confirmation, and distributes to all intended accounts. Skills installed by other tools are shown truthfully first. The user decides adoption per item; adoption retains one source and removes redundant copies.

Deployment: a single-user, single-machine desktop app that may coexist with `npx skills`, Claude synchronization, and plugins. Network access occurs only on an explicit Check or Update.

## Confirmed user decisions (2026-09-29)

| # | Question | Decision |
| --- | --- | --- |
| 1 | Adopt skills installed elsewhere? | Decide per item. Replaces `discover.rs`'s old identify-only rule for real directories. Claude `synced/`, plugins, and Codex built-in `.system/` remain display-only |
| 2 | Original copies after adoption | Keep one source; back up redundant real account directories and same-name Codex shared entries before removal |
| 3 | Tracking | Follow version tags when present, otherwise the main branch. Detect updates and execute update, switch, and rollback |
| 4 | Snapshot retention count | Use a fixed value, not a setting |

## Evidence

| Fact | Basis |
| --- | --- |
| Claude reads only each account's `skills/`, not `~/.agents/skills` | Observed with Claude Code 2.1.285 in isolated HOME |
| Codex reads `$CODEX_HOME/skills` and `~/.agents/skills`; the latter affects all Codex accounts, and both same-name copies load | Observed with codex-cli 0.159.0, `codex debug prompt-input`, working directory outside HOME |
| `npx skills` installs to `~/.claude/skills` and `~/.agents/skills`, recording source, path, and content hash in `~/.agents/.skill-lock.json` | Observed local lock file |
| Some skills include release metadata; archify `skill-release.json` gives repository/version/channel, and its own update hint is read-only | Observed archify 3.0.1 |
| Version schemes vary: main only; version tags (archify `v2.0.0…v3.0.1`); many release branches and `-rc` tags (orca); other branches (`codex/*`, `labs/*`) | Observed `git ls-remote` |
| Sources can be large: archify about 463 MB, orca about 966 MB | Observed GitHub API `size` |
| The previous implementation followed only the default branch, ran a full `git clone` on update, and stored content hashes without commits/tags | Observed `native_skills/apply.rs` `remote_ref` and `skill_bindings` |

## Version model

Source definitions add `track` and `current` (commit plus tag or branch). Old sources without `track` retain main-branch behavior.

| Tracking | Latest-version selection | Use |
| --- | --- | --- |
| Version (`tag`, default pattern `v*`) | Highest semantic-version tag; exclude prereleases such as `-rc` / `-beta` by default | Default when version tags exist |
| Main branch (`default`) | Latest commit on the remote default branch | Default without version tags |
| Specific branch (`branch`) | Latest commit on that branch | User selection |
| Latest matching branch (`branch-pattern`, e.g. `release/*`) | Branch with the highest version in its name | User selection |
| Fixed (`pinned`) | No automatic change; only notify about newer versions | User selection |

- Versions switch at source level and all its accounts follow. Use Make My Version when one account needs different content.
- Checks use only `git ls-remote --heads --tags`, with no download or database write.
- Downloads use partial clone and sparse checkout for `skill_paths` only. On failure, fall back to a shallow clone of the chosen ref and show repository size. Existing full clones remain usable.
- **Check** displays transitions such as `v3.0.1 → v3.0.2`. **Update / switch / rollback** first changes the mirror, then replaces changed active skills for each account using that source; disabled assignments stay untouched. Report the count changed.
- **Preview** (`skills:preview`) fetches the target commit into the mirror cache without checkout, account-directory changes, or database writes. Show added/deleted/modified skill files, affected accounts, and rule changes. Switch, Update To, and source-row Update all preview and require confirmation first.
- Update All follows each source's tracking and leaves pinned sources unchanged. Source rules keep the existing source-update notification behavior.

## Adoption

Identify sources read-only, first match wins:

| Order | Evidence | Result |
| --- | --- | --- |
| 1 | Claude `synced/`, plugin directories, Codex `.system/` | Display only, no adoption |
| 2 | Same-name record in `~/.agents/.skill-lock.json` | Repository = `sourceUrl`; path = containing directory of `skillPath` |
| 3 | `skill-release.json` in the skill directory | Repository = `source.repository`; current version = `version` |
| 4 | Directory or symlink target inside a Git repository | Repository = its `origin`; path = relative path |
| 5 | None of the above | Local-source adoption only (move to `~/my-skills/` first); no version tracking |

After the user selects Adopt for an item:

1. Preview without writes: identified source, recommended tracking, current version. Fetch that version into a temporary directory and compare contents.
2. If contents differ, list changed files and let the user choose source content or preserve edits as a local source. Never overwrite silently.
3. Back up original directories into app data → register source → link to all same-type accounts by default → clean backed-up redundant account directories and Codex shared entries → read back → record operation. Undo restores backups.
4. Group skills from the same repository into one source and download once.
5. Leave the `npx skills` lock file unchanged. If it later adds a shared copy, duplicate detection reports it.

## Codex shared directory and duplicates

- Scan `~/.agents/skills` and label it shared by all Codex accounts, with no per-account disable.
- If a name appears there and in a Codex account's `skills/`, warn that Codex loads two copies. Phase C provides backed-up cleanup with adoption.

## Impact

| Scope | Change |
| --- | --- |
| Skill scan locations | Add `~/.agents/skills`; Claude unchanged |
| Database | Add policy definition fields `track` and `current`; no schema or assignment-table change |
| Snapshots | Keep a few recent versions per source; remove unused older ones |
| Upstream checks | Resolve by tracking instead of `ls-remote HEAD`; remain read-only |
| Update All, Fill, Disable | Update follows tracking; Fill and Disable semantics unchanged |
| Make My Version | `forked_from` records source + commit; original-update detection follows tracking |
| Backups and share links | Include `track`; old data uses main branch |
| User directories | Adoption/cleanup replace or delete real directories only after backup, with undo |
| Other tools | Never write `npx skills`, Claude synchronization, or plugin files |

## Phases

| Phase | Scope | File writes | Acceptance |
| --- | --- | --- | --- |
| A | Scan Codex shared directory; identify sources and duplicates; Other Sources grouping (Unmanaged on Sources since 2026-09-30) | No | Reproduce lock files, real directories, shared directory, and duplicate links in isolated HOME; correct read-back with zero writes; read-only real-environment queries |
| B (implemented) | Tracking, checks, preview, updates, switching, rollback, partial clone | Yes | Isolated HOME: switch/rollback between v3.0.0 and v3.0.1; negative sorting tests (`v2.10.0` vs `v2.9.0`, `-rc`); measure large-repository downloads |
| C (implemented) | Adoption and redundant-copy cleanup, including undo | Yes | Reproduce real layout in isolated HOME; read back one source per account after adoption and byte-for-byte restoration after undo |

## Implementation details

- Adoption preview (`skills:adopt-preview`) fetches only that skill directory into system temporary storage and removes it afterward. For an older local version, compare Git tree IDs against release tags using directory structure without downloading contents, then verify matching files and adopt that version.
- Execution (`skills:adopt`) checks preview fingerprints for local content, target version, and permission to replace edits. Mismatches prevent changes. Backups and manifests live in `adopt-backups/<id>/` under app data; failure and `--undo` restore according to the manifest.
- Merge same-repository skills into an existing source at its current version; adopting one skill must not move the whole source. Link locally renamed skills under their source-directory name.
- The user selects accounts during adoption, defaulting to all (decision 2). Mark accounts that already had the skill, including all Codex accounts for a shared copy. Require at least one; account selection is excluded from the fingerprint.
- Long actions: preview is read-only and Cancel discards results. Execution moves directories and cannot safely stop midway, so it has no Cancel and ignores Esc/backdrop dismissal. Continue in Background closes the dialog while execution continues, with completion notification and undo. Both phases display engine-reported steps (`runs:progress`), using system temporary progress files removed afterward. Version switching also locks the dialog until completion.
- First adoption previews are slower for large sources: orca (about 966 MB, thousands of tags) took about 80 seconds; ordinary repositories about 8 seconds.

## Unverified

- Whether `npx skills` supports a chosen branch or tag.
- Checks/downloads for private repositories using local Git credentials.
- ~~Partial-clone savings on large repositories such as orca~~: observed 2026-09-29. Fetching 3 skill directories from orca's roughly 966 MB repository produced a roughly 35 MB mirror, took about 20 seconds initially, and about 7 seconds for version switching. Fetching only `archify/` from roughly 463 MB produced about 14 MB in about 15 seconds.
- Link behavior on Windows and Linux.
