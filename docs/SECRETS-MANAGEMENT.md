# Secrets Configuration

## Manual configuration

Installation and use require neither the author's secrets repository nor a GitHub account.

1. Open Settings → Secrets and create a file, or enter your own external `.env` path and select Validate and Use. The default creation path is `~/.config/agent-manager/secrets.env`.
2. Enter your API key when adding an API account. Enter MCP variables on Secrets. The page shows variable names and whether they are configured, never values.
3. Confirm the file is connected. If unavailable, repair its path, format, or permissions first. Switching files neither migrates old values nor updates active assistant configuration.

## Use your own private repository

Choose your own repository URL, filename, and hosting platform. Keep it outside this project, for example as below. Existing `mcp.env` and `secrets.env` files can be used without migration.

```text
YOUR_PRIVATE_SECRETS/
  README.md       # Purpose only; no real values
  mcp.env         # Secrets file selected in the app
  mcp.env.example # Placeholders only, for initialization
```

Create a **private repository** on your hosting platform and configure local Git access before cloning outside this project. Installer users can use ordinary `git clone`; source users can also run:

```sh
mkdir -p "$HOME/.config/agent-manager"
bash scripts/init-secrets.sh YOUR_PRIVATE_REPO_URL "$HOME/.config/agent-manager/secrets"
```

Create or edit `mcp.env` there, run `chmod 600 mcp.env` on macOS / Linux, and select it in the app. On Windows, restrict access to your account in file security properties. The engine reads the owner and DACL and rejects unrecognized or broad access; it does not change Windows permissions automatically. Native Windows ACL acceptance remains platform-specific.

Git synchronization is explicit and user-operated. Neither app nor initialization script automatically pulls, commits, or pushes; the script stops if the target directory exists. Resolve pull conflicts in Git before validating again in the app. Disconnecting removes only the file reference, never the repository or credentials.

## .env format

One variable per line. Names use letters, digits, and underscores and cannot begin with a digit; duplicates are rejected. Blank lines, whole-line `#` comments, and an optional `export ` prefix are supported. Double-quoted values parse as JSON strings; single-quoted values are literal. Shell commands are never executed and variables never expanded. Trailing comments become part of the value.

```dotenv
YOUR_PROVIDER_API_KEY="YOUR_API_KEY"
YOUR_MCP_TOKEN="YOUR_TOKEN"
```

Match names to account or MCP references, such as `${YOUR_MCP_TOKEN}`, and replace example values with your own. One app instance currently selects one file. Terminal-only credentials may remain in other files; no merge is needed.

## Publisher secrets

Tauri updater private keys, passwords, and commercial signing certificates belong to maintainers and are injected through GitHub Actions Secrets. Ordinary users do not configure them; installers contain only the verification public key. Maintainers may back up release keys in a separate directory in their own private repository, but must not select it as a user's runtime secrets source.

## Current rules

Real assistant files are authoritative for active configuration. The app database stores only preferences, ownership records, and inactive alternatives.

- Controlled account and MCP credential references require an explicitly selected readable external `.env` file. Missing values, source errors, and malformed references fail closed; they cannot fall back to parent-shell credentials. Agent Manager never writes real plaintext credentials into this repository, SQLite, logs, configuration backups, or test artifacts.
- A credential variable belongs to one account, including its disabled MCP entries. Conflicting references are rejected before registration or writes. Newly created API accounts use an account-specific variable name. Credential-bearing MCP entries cannot be automatically copied between accounts.
- SQLite definition/binding data stores `${VAR}` placeholders; `app_settings` stores only the external path. Status, diagnostics, and logs report variable names, sources, and booleans, never real values.
- An existing selected file must be regular, have no Unix group/other access, contain valid `KEY=VALUE`, and be outside the app data directory. Explicit creation uses mode `600` and never overwrites an existing file. The app does not clone or guess secrets repositories.
- Whether apply writes resolved values into target client configuration depends on the client format; security relies on file permissions and client boundaries.
- Runtime logs and errors must not contain tokens, private keys, full connection strings, or other sensitive values.
- Queries and status perform zero writes. Public `*:plan` and `--dry-run` entry points are retired and rejected.
- Source changes and secret writes respect account session locks. Explicit legacy migration rejects shared ownership and compensates normal write failures; MCP disable no longer performs an implicit credential migration.

## MCP and assistant configuration

MCP uses actual entries in assistant files. `native-manager.db` stores saved but inactive MCP items and persistent variable ownership, never current configuration or credential values; see the [MCP configuration guide](MCP-CONFIG-GUIDE.md). New env `${VAR}` references resolve according to the assistant; headers use native client references. New env literals resembling credentials are rejected. Controlled checks and writes require the selected external file and refuse parent-environment fallback. Literal credentials in an existing entry are not automatically adopted when disabling it.

`secrets:source` stores only the path; `secrets:set` changes only the specified variable after ownership and session checks. Removing an entry or account preserves its variable reservation; a recreated account needs fresh names. The old `secret_values` table supports explicit one-time migration only: the page shows a count without revealing values, and execution requires the confirmation word. Migration rejects conflicting ownership, writes externally without overwriting, and deletes the old table only after success. Normal write failures are compensated. Exports never contain values or external paths; imports reject a nonempty legacy `secrets` section.

Claude Code and Codex default to official `native` installation commands. Brew is shown only as a detected source, never the default. Credentials remain under the corresponding tool and user environment's control.

## Verification and reporting

Use an isolated HOME, `AGENT_MANAGER_DATA_DIR`, fake executables, and fake credentials for acceptance. Never exercise authentication changes against real account directories. Before submitting, run:

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
cd app && pnpm test:unit run && pnpm typecheck && pnpm build
```

If an error appears to contain sensitive information, stop sharing it, remove temporary artifacts, and report only redacted error categories and paths.
