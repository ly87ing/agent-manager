# Agent Manager

Run several Claude Code and Codex accounts side by side. Each account has its own configuration directory and controlled launch command; skills, credential-free MCP servers, and rules can be shared. Built with Tauri 2, React, and Rust/SQLite, the desktop app and CLI share one Rust engine.

The desktop app runs in the foreground: closing its window exits, and command
search loads resources only while open. Navigation does not query every resource
for global badges. See the [lightweight goal](docs/goal-lightweight.md) for the
remaining retirement and memory-bound work.

## Features

**All external sources are optional.** You can manage local accounts, MCP, rules, and existing skills without configuring a repository. The author's public skill repository is only a recommendation and is downloaded after confirmation. Secrets can be entered directly into a local file created by the app or stored in a file from your own private repository; the author's secrets repository is not required.

- **Accounts:** list each assistant's main account (`claude` / `codex`, official login only) and separate accounts such as `cc2` or `ds`. Search by name, command, endpoint, or model, and refresh local status. Each row shows the actual endpoint, model, login state, and quota reset time; expired quota percentages are hidden. Add an official account or an API-provider preset to create its directory and launch command with shared settings and active skills. API keys are stored in an external secrets file and also written in plaintext to that account's assistant configuration when applied; the database stores only references. Edit forms read actual configuration and compare it with the opening baseline before saving; name and connection changes are submitted together. If safe prefill is unavailable, the app explains why and still allows renaming. Removal preserves the directory, login, and sessions; failure to remove a launch command is reported separately. Login copies the official command for you to run in a terminal.
- **Live quota (retirement pending):** the existing status-line action and local quota readers remain until the next lightweight slice. Tray publishing and reminders are already removed.
- **Skills:** manage collections from Git repositories or local directories, with symlinks in each account's skill directory. Active skills are assigned to all accounts by default. Fill links them to accounts where they are missing; disabling one for an account is an exception that Fill respects. Adding a newly discovered skill assigns it to all accounts. Git sources can follow the latest version when version tags exist, otherwise the main branch; a specific branch, matching branches, or fixed version can also be selected. Only used directories are downloaded. Preview changes in Versions before updating or switching to any version. Skills installed by other tools, such as npx skills, appear as unmanaged on Sources and can be adopted individually: retain one source, back up redundant copies before cleanup, and allow undo. Add, update, switch, and remove sources on the Sources page.
- **MCP:** read and write entries in each account's actual configuration files. One row per service summarizes enabled accounts. Fill within one assistant type, or Copy to another assistant. Editing and deletion show file previews; every write checks for external changes. Legacy `_MANAGED_BY` fields have a per-account cleanup preview that preserves the other fields. Test connection runs a separate protocol session and reports the working directory, timeouts, failure stage, natural exit code, sanitized error output, server and tools. It changes no configuration and persists no result; an enabled switch or successful check does not mean the assistant has loaded the service.
- **Rules:** edit each account's `CLAUDE.md` / `AGENTS.md` directly. Templates are local; recommended rules come from skill sources (`rules/` in agent-skills). Fill copies the main account's rules only to accounts without a rule file and never changes existing content.
- **Usage:** read local session records and total tokens per account. Filter by time, account, and model, see the prompt-cache hit rate, and estimate cost from a price table; this is not a bill. Recent sessions list each account's latest conversations (title, directory, time) and copy a command that enters that directory and resumes the conversation with the same account's launch command.
- **Sources:** manage where skills and rules come from and which version they use: add, recommend, select tracking, check and update, switch versions, share, disable, and remove. Adopt account skills with no source from the unmanaged list. Import share links here too.
- **Programs:** manage command-line programs required by skills and MCP, such as wecom-cli and MCP packages fetched with npx. See dependents, install location, version, and available updates. Existing global npm or uv installations stay in place and are upgraded through their original package manager, with a full backup before changes, restoration on failure, and rollback. Missing programs are installed in this tool's directory, one copy per version; switching changes only the pointer. For MCP services fetched with npx / uvx, manage the entry's version and pin unpinned entries with one action. npm install scripts are disabled by default. Preview every action's plan or copy its commands to execute yourself. Assistants themselves show their version and update availability on Accounts; the app does not install them.
- **Settings:** operation history, secrets files, client diagnostics, themes, and configuration backups.

## Experience

- Top navigation: Accounts, Skills, MCP, Rules, Commands, Sources, and Programs. Search `⌘K`, theme controls, Usage, and Settings are on the right. Each page opens with a list, one item per row.
- `⌘K` searches pages, accounts, and resources; selecting an account copies its launch command. `⌘⇧K` lists accounts only. `⌘F` searches the current page; `⌘,` opens Settings.
- No tray window, quota notification, or background account refresh. Application update checks are manual in Settings.
- Light, dark, and system themes; persistent window position and size; single-instance operation.

The application UI is in Simplified Chinese.

## Installation

### Download an installer

[0.1.0 preview installers](https://github.com/ly87ing/agent-manager/releases/tag/preview-v0.1.0-20261008) are available for macOS (Apple Silicon / Intel DMG), Linux (AppImage / DEB), and Windows (NSIS / MSI). The repository is currently private, so downloads require repository access; HTTPS does not bypass access checks. Preview packages do not configure automatic updates.

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

Outputs are in `app/src-tauri/target/release/bundle/dmg/`. Verify the standalone package with `pnpm --dir app run verify:macos`. Linux and Windows installers are built by [.github/workflows/gui-release.yml](.github/workflows/gui-release.yml); the local build script currently targets macOS only. On Windows, launch commands are `.cmd` scripts in `~/.agent-manager/bin`; the tool does not modify the registry. See [Windows support](docs/GUI-PLATFORM-STATUS.md#windows-support-2026-09-30) for scope and verification.

## Quick start

1. Open the app. Accounts lists each assistant's main account and existing separate accounts; reading does not write files. The Getting Started prompt shows remaining steps (log in to the main account; optionally add a skill source) and disappears once completed or dismissed.
2. Select Add Account. For an official account, copy its login command and run it once in a terminal. For an API account, enter its endpoint and API key.
3. Open a new terminal and enter the launch command, such as `cc2`, to use the account alongside others.
4. Select Fill on Skills, MCP, and Rules to assign active resources to the new account. Select Enable Live Quota on Accounts when needed.

AI CLI installation, upgrades, and uninstallation are handled outside this tool.

Apply includes these safety guarantees:

- Preflight validation before writing, with an exact change scope.
- An exclusive lock to prevent concurrent writes.
- External-change detection: fail closed rather than overwrite a target changed outside this tool.
- Conditional rollback and an audit trail, available in operation history for every execution.

Destructive actions, such as removing definitions or clearing logs, require confirmation.

## Data and security

- **Actual configuration is authoritative:** real files and directories in each account are the sole source of truth for active configuration. `native-manager.db` (SQLite) in the app data directory stores preferences, account registration, ownership records, and inactive alternatives such as disabled Claude MCP entries. There is no second writable configuration file. Queries and status reads are read-only and do not create a missing database. Built-in defaults are recommendations applied only after confirmation.
- **Account isolation:** generated launch, login, logout, and status commands bind the current registered account, executable, and child environment. Ambiguous authentication, shared credential ownership, aliased identity files, and identity-changing overrides stop the operation. Stored login labels are local records, not server identity verification. See [account isolation and its security boundary](docs/ACCOUNT-ISOLATION.md).
- **Write scope:** only managed targets are written: managed entries in account configuration directories (owned keys in `settings.json` / `config.toml`, MCP entries, rule files, and skill links) and managed shell launch commands. Main accounts hold official logins only; third-party connections are never written to main configuration. The assistant CLI owns official login and logout; this tool reads local authentication metadata and resolves explicitly selected API/MCP credentials for the selected account.
- **Secrets model:** active MCP entries live in assistant configuration files. This tool stores only saved but inactive items, such as disabled Claude entries. Credentials become `${VAR}` references with values moved to an external file; ordinary values that do not resemble credentials, such as `NODE_ENV=production`, stay unchanged. After you explicitly select an external `.env` file on Secrets, the app checks that it is a regular file with mode `600` and valid `KEY=VALUE` contents. Entered or adopted values are written only there. The page never displays values; the database stores only the file path, and configuration backups include neither paths nor values. Writes fail closed without a usable selected file.
- **No data collection:** no telemetry uploads. Logs stay in the local app data directory, are strictly redacted, and are removed according to the retention setting.

## Documentation

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
