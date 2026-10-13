# Launch Configurations

## Responsibilities and sources of truth

The `launch` domain is independent of assistant accounts and MCP secrets. Its
structured recipe contains a platform, shell, application or executable target,
literal argument list, and one or more explicitly named time-zone variants.
`la` and `sg` are examples, not required names or a fixed list of time zones.

## Runtime path

`launch:build --spec <JSON>` returns each complete launch command and shortcut
definition. The CLI and desktop bridge use the same Rust renderer. Building does
not read shell files, load secret sources, create HOME/data, or execute commands.
POSIX alias definitions quote the complete command as well as each argument.
Windows output uses PowerShell and a child-specific ProcessStartInfo environment.

`launch:status --shell zsh|bash|powershell` reads actual shortcuts and their file
baseline. `launch:preview --spec <JSON> --base <baseline>` returns complete file
differences without writing. `launch:apply` requires the same baseline plus
`--confirm`. Removal uses `--remove <name>` instead of `--spec`.

Zsh writes only `$ZDOTDIR/.zshrc` (or `HOME/.zshrc`), Bash writes `HOME/.bashrc`,
and Windows writes individual `HOME/.agent-manager/launchers/<name>.ps1` scripts.
Windows scripts are invoked by their displayed full path; PATH and execution
policy are never changed. The actual files are authoritative; there is no second
database copy of active shortcut commands.

## Constraints and failure behavior

- Unknown fields, duplicate names, incompatible platform/shell combinations, and
  control characters are rejected before work begins.
- An application need not be installed to generate a command for another machine.
- macOS applications use `open --env`; Linux and CLI commands receive a scoped
  environment. Windows output injects an environment variable, not a system time
  zone. Each application's handling of `TZ` needs separate verification.
- The old `agents`, `accounts`, `profiles`, and `aliases` APIs remain retired.
- New zsh shortcuts will be maintained only in `.zshrc`, never `.zprofile`.
- Outside-block aliases and unmarked Windows scripts cannot be overwritten.
  Preflight rejects stale baselines before creating state. Under the apply lock,
  the plan is recomputed and each write uses atomic byte comparison. Audit
  failures conditionally restore written bytes without overwriting external edits.
- Audits contain names and file digests, never command bodies or arguments.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_launch
cargo test --manifest-path engine-rs/Cargo.toml --lib native_launch
cargo test --manifest-path engine-rs/Cargo.toml --test native_launch_shortcuts
```

The literal-argument regression executes generated aliases through Bash and zsh
when installed, including apostrophes, spaces, empty arguments, and command-like
text. Windows rendering tests are not native Windows application acceptance.
