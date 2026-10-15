# Launch Configurations

## Responsibilities and sources of truth

The `launch` domain is independent of assistant accounts and MCP secrets. Its
structured recipe contains a platform, shell, application or executable target,
literal argument list, generic environment-variable list, and one or more named
variants. Each variant can optionally set `TZ`; `la` and `sg` are examples, not
required names or a fixed list of time zones. A variant's time zone overrides a
recipe-level `TZ` variable so regional shortcuts remain concise.
New configuration starts with one Singapore variant (`Asia/Singapore`). Users can
change it or add other variants; opening a saved recipe preserves its variants.

Saved recipes live in the `launch_recipes` table of `native-manager.db`. They are
inactive application data. Only the recipe is stored; complete commands are
rendered from it when read. Saving does not require the target to be installed,
inspect or change shell files, or start a process.

[Configuration backups](extension-backups.md) include saved recipes as an optional,
selectable module. Restore validates recipes and preserves their identifiers;
it saves inactive data without launching applications or writing shell files.

Launch is the first top-level navigation item and the default desktop opening
page. It initially shows only saved entries, so common graphical applications can
be started immediately and terminal commands can be copied without opening an
editor or Settings. Each saved variant occupies one table row with its full
target path, shortcut name, time zone and primary launch/copy action. Other
actions live in the row menu. New configuration, clicking a target path, or the
row's Edit configuration action opens a modal editor. Its form and generated
commands scroll independently of the fixed action footer. Cancel or Escape
discards the draft and returns focus to the original trigger without moving the
list. The retained Settings entry routes to this same top-level page.
Successful saves update the list and close the editor. Failed saves keep the
editor open with the draft and error details available for correction.

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
Windows command recipes can resolve bare batch names through the calling terminal's
PATH, excluding aliases and PowerShell functions; native executable lookup remains
unchanged. Batch targets (`.cmd` / `.bat`)
use the system `cmd.exe` with AutoRun and delayed expansion disabled, batch-specific
literal argument quoting, and the same child-only environment. Commands beyond
the 8,191-character batch limit fail before execution. Conventional `%*` forwarding
is covered; a batch script can still reinterpret arguments in its own body.
Batch paths with quotes, surrounding whitespace or trailing dots are refused
before execution; those aliases cannot fall through to native argument quoting.

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
Row controls use concise labels: Launch, Edit, Copy command, Copy alias/function,
and Delete. The complete alias/function definition is still copied.
The operation refuses an already-running target and never quits or restarts it.
Windows checks executable names
conservatively; Linux checks `/proc` executable paths; macOS checks bundle process
paths. A launch request does not prove the application accepted its environment.

Launched GUI applications do not belong to the manager's temporary command jobs
and survive its exit. Command-line programs are copied to the user's terminal for
normal interactive input and output. Launch audits record save/delete/run events,
never process arguments, rendered commands, or definitions.
An actual run records `requested` before the system call, then `accepted` or
`failed` with the available system exit code. Acceptance means the system accepted
the request, not that the application loaded or honored its environment. A failed
completion audit preserves the actual outcome: accepted requests return
`audit_recorded:false` and a warning asking the user to inspect the application
before retrying; failed requests retain their launch error and disclose the
recording failure. A request-audit failure prevents the system call.

## Constraints and failure behavior

User-triggered action failures immediately show a dismissible viewport toast for
ten seconds, with a translated action title and the original diagnostic. Editor
notifications appear at the top so they cannot cover its action footer. Details
for launch and copy failures appear only in the toast and disappear on dismissal
or expiry; saved and preview entries do not acquire persistent error rows.
Form failures remain in the editor's fixed action footer, and delete failures
remain inside the confirmation. Read failures appear beside the
saved-list controls. A new action, draft change or page exit clears stale action
feedback. No error path scrolls the page or retries a launch automatically.
Catalog and saved-list read errors remain independent. Read again retries both
requests; one successful response cannot clear the other request's failure.
Save/delete/request success is transient too. All action feedback uses the shared
[interface feedback](interface-feedback.md) policy; form errors retain their
local correction context.

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
native Windows application acceptance. On Windows, the execution regression runs
generated functions through Windows PowerShell and, when available, PowerShell 7.
It reads back native executable and batch-shim arguments, child environment and
nonzero exit status, including PATH lookup and special characters in file paths.
Runtime cases read back failed system requests and accepted requests whose
completion audit failed. Frontend cases cover either asynchronous read ordering,
full retry recovery and the accepted-request audit warning.
