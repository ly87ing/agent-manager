# Agent Manager

Manage MCP servers, rules, skills, and their extension dependencies for existing assistant configuration directories. Build general launch commands with chosen applications or terminal commands, environment variables, arguments, optional time zones and shortcut names. Built with Tauri 2, React, and Rust/SQLite, the desktop app and CLI share one Rust engine.

**Agent Manager does not manage accounts.** It cannot create, edit, delete, switch, launch, log in to, or log out of an account; change its credentials, provider, model, or runtime settings; or create account-bound launch commands. Historical target registrations are read-only metadata used to locate extensions. See the [extension-only contract](docs/EXTENSIONS-ONLY.md).

## Features

- **Assistants:** list known assistant extension targets and their registered paths. This view is read-only and does not check installation, login identity, or credentials.
- **Skills:** manage collections from Git repositories or local directories and their links in existing assistant skill directories. New personal collections default to `~/skills/<collection>`, with custom locations and existing paths preserved. Fill missing links, disable individual assignments, preview versions, and adopt unmanaged skills after confirmation. See [skill directories and source switching](docs/subsystems/skills.md).
- **MCP:** read and edit entries in actual assistant configuration files, with previews, external-change checks, and per-target actions. Shared JSON files preserve every byte outside the MCP container. Connection checks run a separate protocol session and do not establish that an assistant has loaded the service.
- **Rules:** edit existing targets' `CLAUDE.md` / `AGENTS.md`, use local templates, or apply recommended rules from skill sources. Fill only targets without a rule file.
- **Sources:** manage optional skill and rule repositories, versions, local sources, and resource share links. No external repository is required for local extension management.
- **Programs:** manage command-line dependencies used by skills and MCP. Assistant installation and updates remain outside this tool.
- **More tips:** read-only guidance about independent environments and shell aliases in Settings. Copying a generic example does not execute it or edit files.
- **Launch configurations:** Settings → Launch configurations saves an application or terminal command, generic environment variables, literal arguments, optional time zones and shortcut names in Agent Manager. Saved graphical applications can be launched directly; terminal commands provide complete commands and alias/function definitions for manual use. Agent Manager does not write shell configuration. See [launch configurations](docs/subsystems/launch-configurations.md).
- **Settings:** English by default, with Simplified Chinese available under Language; themes, operation history, MCP-specific secret files, extension configuration backups, and manual application updates. Account credentials and unrelated secret variables are read-only.

## Experience

Navigation contains Skills, MCP, Rules, Sources, Programs, Assistants, and Settings. `⌘K` and `⌘⇧K` search extension resources and pages; `⌘F` searches the current page; `⌘,` opens Settings. General launch configurations are available under Settings and search. Account pages remain removed.

The app runs in the foreground and exits when its window closes. Search queries run only while search is open. Usage, quota, recent-session collection, tray residency, and automatic application update checks remain retired. See [resource bounds](docs/subsystems/resource-bounds.md).

## Installation

### Download an installer

Historical [0.1.0 preview installers](https://github.com/ly87ing/agent-manager/releases/tag/preview-v0.1.0-20261008) are available for macOS (Apple Silicon / Intel DMG), Linux (AppImage / DEB), and Windows (NSIS / MSI). The repository is currently private, so downloads require repository access; HTTPS does not bypass access checks. Preview packages do not configure automatic updates and predate the extension-only change; they do not prove the current source behavior.

A production release does not require purchasing an Apple developer account. By default, macOS uses ad-hoc signing and Windows has no Authenticode signature, so the operating system may require manual installation confirmation. Update packages still require signing; commercial certificates and Apple notarization can be enabled separately. See [platform status](docs/GUI-PLATFORM-STATUS.md) for release and acceptance status.

### Build from source

Prerequisites: the Rust toolchain, Node.js, and pnpm. Building and using the app do not require the author's secrets repository. Enter secrets manually in the app or select an `.env` file from your own private repository; see [secrets configuration](docs/SECRETS-MANAGEMENT.md).

Clone without `--recursive`.

macOS:

```sh
git clone https://github.com/ly87ing/agent-manager.git
cd agent-manager
pnpm --dir app install --frozen-lockfile
pnpm --dir app run build:macos
```

Outputs are in `app/src-tauri/target/release/bundle/dmg/`. Verify the standalone package with `pnpm --dir app run verify:macos`. Linux and Windows installers are built by [.github/workflows/gui-release.yml](.github/workflows/gui-release.yml); the local build script currently targets macOS only. Historical launch scripts are left untouched and their controlled launcher now refuses execution. See [Windows support](docs/GUI-PLATFORM-STATUS.md#windows-support-2026-09-30) for scope and verification.

## Quick start

1. Prepare and authenticate the assistant with its own official tools.
2. Open Agent Manager and manage skills, MCP, or rules for an existing target. Optional repositories are added on Sources.
3. If an MCP service needs a secret, select a dedicated external `.env` file and fill only its MCP-specific variables.
4. Review file previews before applying changes. The assistant itself controls when changed extensions are loaded.

AI CLI installation, accounts, authentication, account-bound launches, and runtime settings are handled outside this tool. General launch recipes are independent of accounts and never change the system environment or time zone. An application may ignore injected variables; verify its behavior separately.

Apply includes these safety guarantees:

- Preflight validation before writing, with an exact change scope.
- An exclusive lock to prevent concurrent writes.
- External-change detection: fail closed rather than overwrite a target changed outside this tool.
- Conditional rollback and an audit trail, available in operation history for every execution.

Destructive actions, such as removing definitions or clearing logs, require confirmation.

## Data and security

- **Actual configuration is authoritative:** real extension files determine active extension state. `native-manager.db` stores preferences, immutable historical target metadata, ownership records, inactive extension alternatives, and saved launch recipes. Status reads do not create a missing database.
- **Account boundary:** account commands return `FEATURE_RETIRED` before loading credentials, preflight, or locks. Policy writes and backup imports cannot alter historical account or alias definitions; exports omit them. Existing account directories, credentials, sessions, and launch scripts are preserved.
- **Write scope:** MCP containers, rule files, skill links, extension dependencies, saved launch recipes, and the tool's own extension state. Launch definitions are copied for the user to apply manually. Shared configuration files preserve non-MCP content. Ambiguous paths, identity aliases, hard links, and external changes fail closed.
- **Secrets:** only variables referenced by MCP may be edited, excluding reserved assistant credentials and variables referenced by historical account metadata. Account files cannot be selected as secret sources. Backups exclude secret values and source paths. Legacy database secret migration is retired because its contents may include account credentials.
- **Local data:** logs stay in the application data directory and are redacted and retained according to settings.

## Documentation

- [Extension-only scope](docs/EXTENSIONS-ONLY.md): current product boundary and verification
- [Architecture](docs/ARCHITECTURE.md): data flow and call boundaries
- [MCP configuration guide](docs/MCP-CONFIG-GUIDE.md): reading, writing, enabling, disabling, and secrets rules
- [Secrets boundaries](docs/SECRETS-MANAGEMENT.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Platform status](docs/GUI-PLATFORM-STATUS.md): platform builds and acceptance
- [Changelog](CHANGELOG.md)
- For contributors: [GUI redesign contract](docs/GUI-REDESIGN.md), [goals and acceptance](docs/goal-rust-gui.md), [engine contract](contract/README.md), and [desktop development](app/README.md)

## Development

Download cross-platform installers from successful [GUI CI](https://github.com/ly87ing/agent-manager/actions/workflows/gui-ci.yml) runs (Artifacts, retained for 14 days), or use **Run workflow** to build a selected branch manually.

| Platform | Artifacts |
| --- | --- |
| macOS Apple Silicon / Intel | Separate DMGs |
| Windows x64 | NSIS EXE, MSI |
| Linux x64 | AppImage, DEB |

Each artifact includes `SHA256SUMS.txt` and `build-info.json` to check file digests, commit, and target architecture. These are test installers without production signing. Production signing, notarization, and automatic updates use GUI Release; ordinary CI does not create a Release.

Repository layout: `engine-rs/` (the Rust library and SQLite storage shared by CLI and desktop), `app/` (Tauri and React), `contract/` (Engine Contract v1 schemas), and `scripts/` (build and acceptance scripts).

Verification commands, from the repository root:

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
cd app && pnpm test:unit run && pnpm typecheck && pnpm build
```

`./scripts/ci-smoke.sh` runs smoke tests in an isolated data directory without touching the real environment. Full verification requirements are in the Verification section of [docs/goal-rust-gui.md](docs/goal-rust-gui.md).

## License

[MIT](LICENSE)
