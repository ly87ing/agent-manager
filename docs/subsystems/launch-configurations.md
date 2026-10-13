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

## Constraints and failure behavior

- Unknown fields, duplicate names, incompatible platform/shell combinations, and
  control characters are rejected before work begins.
- An application need not be installed to generate a command for another machine.
- macOS applications use `open --env`; Linux and CLI commands receive a scoped
  environment. Windows output injects an environment variable, not a system time
  zone. Each application's handling of `TZ` needs separate verification.
- The old `agents`, `accounts`, `profiles`, and `aliases` APIs remain retired.
- New zsh shortcuts will be maintained only in `.zshrc`, never `.zprofile`.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_launch
cargo test --manifest-path engine-rs/Cargo.toml --lib native_launch
```

The literal-argument regression executes generated aliases through Bash and zsh
when installed, including apostrophes, spaces, empty arguments, and command-like
text. Windows rendering tests are not native Windows application acceptance.
