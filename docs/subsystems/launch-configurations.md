# Launch Configurations

## Responsibilities and sources of truth

The `launch` domain is independent of assistant accounts and MCP secrets. Its
structured recipe contains a platform, shell, application or executable target,
literal argument list, generic environment-variable list, and one or more named
variants. Each variant can optionally set `TZ`; `la` and `sg` are examples, not
required names or a fixed list of time zones. A variant's time zone overrides a
recipe-level `TZ` variable so regional shortcuts remain concise.

Saved recipes live in the `launch_recipes` table of `native-manager.db`. They are
inactive application data. Only the recipe is stored; complete commands are
rendered from it when read. Saving does not require the target to be installed,
inspect or change shell files, or start a process.

Launch is the first top-level navigation item and the default desktop opening
page. It initially shows only saved entries, so common graphical applications can
be started immediately and terminal commands can be copied without opening an
editor or Settings. New configuration and Open configuration explicitly reveal
the editor. The retained Settings entry routes to this same top-level page.

Environment remediation can open a new unsaved draft with the selected
collection's time zone and locale variables. The application target stays empty
until the user chooses it. Leaving the editor consumes this one-time handoff;
returning to Launch does not recreate it. No build, save or run occurs during
navigation, and existing saved recipes are preserved.

## Runtime path

`launch:build --spec <JSON>` returns each complete launch command and definition.
The CLI and desktop bridge use the same Rust renderer. Building does not read
shell files, load secret sources, create HOME/data, or execute commands. POSIX
output provides a complete alias definition. Windows output provides a named
PowerShell function that can be pasted into `$PROFILE` or the current session.

`launch:list` reads saved recipes without creating a database or table.
`launch:save --spec <JSON>` creates a saved recipe. Updating uses the returned
`--id` and `--base` revision; `launch:delete --id <id> --base <revision>` removes
only that saved recipe. Both mutations compare the revision inside a database
transaction and record an audit without command bodies. A stale edit or failed
audit leaves the saved recipe unchanged. Recognized credential literals cannot
be saved. Recipes for another platform and programs that are not installed can
be saved and copied.

The desktop exposes separate actions to copy one launch command, copy one complete
alias/function definition, or copy all definitions. Saved entries retain these
copy actions without reopening the editor. Users decide whether and where to
paste the result. Agent Manager has no `launch:status`, `launch:preview`, or
`launch:apply` operation and does not write `.zshrc`, `.bashrc`, `.zprofile`,
PowerShell profiles, PATH, or script execution settings.

`launch:catalog` reports the local platform, a default shell, and installed macOS
application bundles. Other executable paths can be chosen in the desktop file
picker or entered manually. A saved local graphical application's variant has a
direct Launch action. That click supplies the explicit `--confirm` marker to
`launch:run --spec <JSON> --name <variant> --confirm`; no second dialog is needed.
The operation refuses an already-running target and never quits or restarts it.
Windows checks executable names
conservatively; Linux checks `/proc` executable paths; macOS checks bundle process
paths. A launch request does not prove the application accepted its environment.

Launched GUI applications do not belong to the manager's temporary command jobs
and survive its exit. Command-line programs are copied to the user's terminal for
normal interactive input and output. Launch audits record save/delete/run events,
never process arguments, rendered commands, or definitions.

## Constraints and failure behavior

User-triggered action failures immediately show a dismissible viewport toast for
ten seconds, with a translated action title and the original diagnostic. Details
also remain beside the affected saved/preview entry, in the editor's sticky
action area, or inside the delete confirmation. Read failures appear beside the
saved-list controls. A new action, draft change or page exit clears stale action
feedback. No error path scrolls the page or retries a launch automatically.

- Unknown fields, duplicate alias or environment-variable names, non-portable
  environment-variable names, incompatible platform/shell combinations, and
  control characters are rejected before work begins.
- An application need not be installed to save or generate a command for another
  machine. Only a local graphical application can be launched from the UI.
- macOS applications use repeated `open --env` options; Linux and CLI commands
  receive a scoped environment. Windows output injects each variable into the
  child process. None of these change the system environment or time zone, and
  each application's handling of the variables needs separate verification.
- The old `agents`, `accounts`, `profiles`, and `aliases` APIs remain retired.
- Saving and copying cannot modify a shell file. A user who pastes a definition
  into shell configuration owns that change outside Agent Manager.
- Audits contain saved recipe identifiers and revisions, never command bodies or
  arguments.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_launch
cargo test --manifest-path engine-rs/Cargo.toml --lib native_launch
cargo test --manifest-path engine-rs/Cargo.toml --test native_launch_runtime
cargo test --manifest-path engine-rs/Cargo.toml --test native_launch_saved
pnpm --dir app test:unit --run src/test/LaunchPage.test.tsx
```

The literal-argument regression executes generated aliases through Bash and zsh
when installed, including apostrophes, spaces, empty arguments, command-like
text, and generic environment variables. The runtime regression reads back the
scoped environment from a real child process. Saved-recipe tests restart the CLI
and read the recipe back while checking that shell files are unchanged and no
target process starts. Windows rendering tests are source-level evidence, not
native Windows application acceptance.
