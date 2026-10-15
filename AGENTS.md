# AGENTS.md

## Current goal

- The independent environment tool inspects macOS regional settings and the caller's process environment. Environment profiles are maintained independently in Agent Manager's database. Comparisons remain read-only, including profiles with a legacy automatic-correction flag, and may query the selected profile's time server. Applying a profile is a separate preview and confirmation. `environment:sync-time --confirm` may adjust only the system clock, after a fresh clearly excessive offset and macOS authorization, followed by independent verification. Preserve unknown evidence and process boundaries. See [Environment checks](docs/subsystems/environment-checks.md).

- Environment remediation can open only the date/time or language/region settings pane, prefill an unsaved general launch recipe, or read the automatic-time switch through `environment:check-time-settings --confirm`. Authorized inspection stays read-only and never invokes correction. Keep mismatch explanations and scope visible without source details; do not change the checker process to make a result match.

- Implement a Tauri 2 interface and a native Rust backend for MCP, rules, skills, their extension dependencies, and user-controlled general launch configurations. Launch recipes are saved only in Agent Manager, generate copyable commands and alias/function definitions, and can explicitly launch a local graphical application. Agent Manager never writes those definitions to shell configuration. This feature does not restore account management. [docs/EXTENSIONS-ONLY.md](docs/EXTENSIONS-ONLY.md) takes precedence over earlier account designs. Data semantics remain in `docs/GUI-REDESIGN.md`; acceptance history is in `docs/goal-rust-gui.md`.
- Never manage accounts: no creation, registration edits, removal, switching, login/logout, account-bound launch commands, credentials, provider/model/runtime changes, or assistant lifecycle actions. General launch configurations never inspect account identity or credentials. Historical account registrations are immutable resource-location metadata. Preserve real account files, sessions, directories, and existing scripts.
- The desktop app and CLI reuse the same Rust library. The new version does not depend on Python, `manage.sh`, legacy adapters, or the source directory.
- Python is no longer maintained, and the user has authorized its removal. Do not add Python features, projection compatibility layers, or obligations to compare two implementations.
- A goal is not an implemented state. Do not call a candidate complete until it has passed compilation, actual operation, and independent acceptance.

## Implementation collaboration

- The lead reviewer owns design, scope decisions, and acceptance. When implementation is delegated, an authorized implementation tab writes product code and tests, and the lead reviewer independently verifies it. If no implementer is available, the lead reviewer implements and fully verifies the work.
- Kimi currently has no implementation permission. Do not assign it work or repeatedly retry. Consider it again only after permissions are restored and the user explicitly selects it again.
- Deliver one small end-to-end slice before expanding scope. Each shared interface has one owner. Give implementers the file scope, inputs, outputs, and negative verification cases.
- The user has authorized incremental commits and pushes of accepted changes on `main`. The lead reviewer commits, checks the entire outgoing range and remote commits, and excludes unrelated changes. This authorization does not include creating Releases, tags, or publishing installers.
- Use English for repository documentation and new content. Commit and push verified changes promptly; conversation remains in Simplified Chinese.

## Development and verification

- The Rust toolchain, Node.js, and pnpm are the development tools for the native version. Do not make Python or the old make pipeline prerequisites.
- Before changing behavior or shared interfaces, read history, callers, and nearby tests, and use `change-discipline`. Use `solution-shaping` for designs and migrations, `verification` for acceptance (including UI), `reader-facing-writing` for documentation, and `artifact-hygiene` for temporary artifacts.
- Before changing code, read [lessons](docs/LESSONS.md) and the [subsystem reference](docs/subsystems/README.md) for the area; add a lesson when a fix had to be redone or a check passed without exercising the change, and update the reference in the same commit when a documented contract changes.
- UI changes follow [UI rules](docs/UI-RULES.md); check every page a change touches at about 1000 px and 760 px.
- Run the smallest relevant check first, then the corresponding full checks: `cargo test --manifest-path engine-rs/Cargo.toml`, `cargo test --manifest-path app/src-tauri/Cargo.toml`; in `app/`, run `pnpm test:unit run` and `pnpm build`.
- For documentation changes, check links, commands, and consistency with the goals. Do not restore retired requirements to satisfy assertions about old wording. State explicitly when verification was not run or did not exercise the change.
- Do not delete tests, weaken assertions, or skip checks to conceal valid failures. Remove Python-only tests with the retired implementation; migrate still-applicable data safety, scope isolation, and negative cases to Rust.
- Real acceptance uses an isolated HOME, data directory, and test-assistant paths. Mocks, exit codes, and implementer reports cannot replace reading back actual results.

## Data and cleanup boundaries

- Assistants' real files and directories are the sole source of truth for active configuration. `native-manager.db` stores only app preferences, recognition definitions, ownership records (location + digest), and saved but inactive alternatives that the real environment cannot represent (see "Data model: the actual configuration is authoritative" in `docs/GUI-REDESIGN.md`). Do not add a second `settings.json` or other writable configuration for Agent Manager. Queries, previews, cancellation, and rehearsals must not write state.
- UI reads and form prefill use actual configuration. Editing an active item writes directly to its real file after re-reading and comparing it before writing. Before switching to an alternative, save the actual contents back. Built-in defaults are recommendations only: apply them after individual confirmation, never persist them automatically or count them as pending synchronization.
- Saving an alternative does not install software or apply resources. Actions for one assistant must not affect another or overwrite external changes.
- Before removing legacy implementations, inventory callers, CI, packaging, documentation, and test references. Preserve uncommitted changes as a local patch. Do not run bulk reset/clean or delete user sessions, real configuration, credentials, or external repositories.
- Commit code cleanup and behavior changes separately. Do not rewrite history.

## Generated files

- Recommended rules are not maintained in this repository. Their text lives in `rules/` in the agent-skills repository; this tool reads it as a skill source and writes to targets only when the user applies individual items. Active rules are the files used by each assistant. `engine-rs/assets/legacy-rules/` contains sanitized fallback copies for legacy database migration. Preserve original-default fingerprints when retiring text; do not edit these copies as recommendations.
- Keep the README and task documents consistent with actual acceptance status. Do not claim a result passed unless it was generated, run, and read back.

## Sensitive information

- Real sensitive information belongs only in an external private `secrets/` repository. Never put it in this repository's code, documentation, examples, plans, HTML, README, or generated artifacts.
- In Tailscale, SSH, and remote-access scenarios, treat real usernames, device names, `.ts.net` domains, Tailscale IPs, SSH Host configuration, public-key contents, and combinations that reveal internal topology as sensitive.
- This repository may contain only placeholders, templates, injection logic, and generic guidance. Use `YOUR_*` in examples. Databases and logs must not store plaintext credentials.
