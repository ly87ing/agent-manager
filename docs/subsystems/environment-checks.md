# Environment checks

## Responsibilities and sources of truth

The Environment page inspects macOS regional settings and the caller's process
environment. Targets are optional and grouped into user-created named collections.
Blank targets provide observations without a comparison. No country, time zone,
language, locale or time server is a product default.

The desktop and CLI share `environment:check` in
[native_environment](../../engine-rs/src/native_environment/mod.rs). It returns
before secret-source loading, database access, locks or operation logs. Checks
never change system time, preferences, shell files or assistant resources.

Sources are `/etc/localtime`, the current user's `AppleLanguages` and `AppleLocale`
through Foundation preferences, selected regional-format preferences,
`/etc/ntp.conf`, `systemsetup -getusingnetworktime`, the running `timed` service,
and the read-only Darwin `ntp_gettime` kernel interface.
Only `TZ`, `LANG`, `LC_ALL` and the six macOS POSIX locale categories are read from
the caller's environment. System local time is read without a process `TZ`
override. Every observation names its source.

Language comparison uses the complete ordered list, ignoring tag casing.
Region uses the country component of `AppleLocale`. Each process locale category
uses nonempty `LC_ALL`, that category, nonempty `LANG`, then `C`; UTF-8 and UTF8
spellings compare equally. Unset `TZ` inherits the system zone; empty `TZ` means
UTC. Time-zone targets use explicit IANA names.

The desktop also enables strict raw-locale comparison when a locale target is
entered. This separately compares every explicitly set nonempty locale variable,
including values shadowed by `LC_ALL`; unset variables are allowed. The checkbox
can disable this extra comparison. CLI callers enable it with `strict_locale:true`.

Kernel time synchronization is collected locally without administrator access
or a network query. The optional `time_synchronized` target compares the kernel's
reported synchronized/unsynchronized state. `time_sync` contains the raw state
code and kernel maximum/estimated errors in milliseconds; a read failure or an
unrecognized state is null and the corresponding rows stay unknown. `TIME_ERROR`
means unsynchronized or a clock error. Darwin can return it as `-1/EIO` after
populating the output, so the reader validates both the return and output state.
See Apple's [XNU implementation](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.61.5/bsd/kern/kern_ntptime.c).

Kernel errors are inventory values, separate from the externally measured offset
and its tolerance. Kernel state does not establish the most recent successful
network synchronization time, the selected server, or actual current accuracy.
The implementation does not parse private `timed` logs or cached state to infer
these facts.

## Runtime path

Opening Environment reads local observations and saved collections without any
network query or write. Selecting a collection clears the old report. Check
selected collection compares its values and queries its explicit time server,
when configured. The destination and automatic-correction behavior are disclosed
before checking. No collection is selected by default.

An explicit check immediately shows a dismissible completion toast with target
counts and the names of differences or unknown checks. Long lists show three
names per category and a remaining count; the persistent result summary keeps
the complete lists. Matching targets are stated as matching selected criteria,
with untargeted unknown observations disclosed separately. Inventory-only checks
never claim an all-clear. Failed requests and unconfirmed time correction do not
show success. Starting a new check, selecting a collection or leaving the page
clears the previous notification. Opening the page does not announce completion.
Collection save/delete, settings opening and inspection outcomes use the shared
[interface feedback](interface-feedback.md) policy. Transient action messages do
not add permanent status rows; comparison and correction evidence remain in the
report.

Add/Edit opens the criteria editor; Use current values as targets explicitly
prefills its fields without asserting those values are correct. Save persists
the name, target and optional `auto_sync` flag in `environment_profiles` inside
the existing `native-manager.db`. It does not apply settings. Missing database
or table reads as an empty list without creating either. Save/delete use an
Immediate transaction and revision-based compare-and-swap, preserving concurrent
edits. Duplicate names are rejected. No separate settings file or account state
is involved. The collection endpoints are `environment:profiles`,
`environment:save-profile --spec JSON [--id ID --base SHA256]` and
`environment:delete-profile --id ID --base SHA256`.

```sh
cargo run --manifest-path engine-rs/Cargo.toml -- environment:check --json
cargo run --manifest-path engine-rs/Cargo.toml -- environment:check --spec '{"timezone":"Europe/London","languages":["en-GB"],"region":"GB","locale":"en_GB.UTF-8","strict_locale":true,"ntp_server":"pool.ntp.org","automatic_time":true,"time_synchronized":true,"max_clock_offset_ms":2000}' --json
```

The CLI describes its calling terminal's inherited environment; the desktop
describes Agent Manager's process. Neither executes shell startup files or
establishes another application's effective settings.

Mismatch and unknown rows show their explanation and scope without opening source
details. Process differences offer **Create launch configuration**, which opens
the existing launch editor with the collection's optional time zone and locale.
Locale prefill covers `LANG`, `LC_ALL` and the six supported categories. The draft
has no application target and is neither saved nor run automatically. Users choose
the application, review the command and explicitly save or launch it. Existing
processes and shell files remain unchanged; other applications require their own
checks. A shadowed raw-locale conflict is distinguished from an effective mismatch.

System time/region differences offer **Open settings**. The shared
`environment:open-settings --pane date-time|language-region` endpoint accepts only
these two destinations and opens the macOS settings pane. Users make the change
there and recheck; opening the pane does not establish a successful adjustment.

An unreadable automatic-time switch offers **Check with administrator
authorization**. A read-only confirmation shows the fixed
`/usr/sbin/systemsetup -getusingnetworktime` command and any reference-time query
destination before `environment:check-time-settings --confirm [--spec JSON]
[--probe-time]`. It reads that switch with macOS authorization, then collects a
fresh report. This path never invokes clock correction, even for a collection
with `auto_sync` enabled. Cancellation or failure retains the previous report;
neither is a successful read. No credentials are handled or stored by the app.
Background peer identity stays unknown and has no repair action.

The CLI explicitly adds `--probe-time`. The page's Check selected collection
action discloses the saved time server and UDP port 123. CLI queries use the target server, or the
configured server when no target is supplied. Name lookup and UDP response each
have a three-second deadline. One request is sent, with no retries; the desktop
process enforces a four-second interval. No scheduled queries run. Separate CLI
processes do not share this interval, so users must retain it.

A response must be a synchronized NTPv3/v4 server reply with matching originate
timestamp, usable stratum and receive/transmit timestamps. Results include offset,
round trip, uncertainty, stratum and the queried address. Offset plus uncertainty
must fit the tolerance to match. An uncertainty interval crossing the tolerance
boundary is unknown. This unauthenticated measurement never adjusts the clock or
proves which server `timed` currently uses.

## Time correction

`environment:sync-time --spec JSON --confirm` is the only clock-writing endpoint.
Its target must explicitly include a valid `ntp_server` and
`max_clock_offset_ms`. A collection's `auto_sync` defaults to false; enabling it
selects this endpoint when the user clicks Check selected collection. Selecting
or saving the collection never corrects time. No background schedule runs.

The endpoint makes a fresh independent measurement. A matching offset returns
`not_needed`; missing or overlapping evidence returns `unavailable`. Neither
case requests privileges. A clear mismatch invokes macOS's `/usr/bin/sntp` once
against the measured IP using a fixed AppleScript and administrator authorization.
The program path, options and clean environment are fixed; the only argument is
a parsed IP quoted as data. Passwords are handled by macOS, never by Agent Manager.
No privileged helper/service is installed and no authorization policy is changed.
See Apple's [administrator authorization guidance](https://developer.apple.com/library/archive/technotes/tn2065/_index.html).

SNTP uses `-S -s -n 1 -t 5`; it may step the clock or request a gradual adjustment.
The authorization subprocess has a 150-second deadline. A second independent
measurement follows correction, cancellation or failure. Query intervals are at
least four seconds within this flow, including around the privileged request.
Only an applied command with a matching new measurement returns `verified`.
Other outcomes are `unverified`, `cancelled` or `failed`; an exit code alone
cannot establish success. Results include the pre-correction offset and
uncertainty in `time_repair`.

The clock is the only system setting this endpoint may change. Time zone,
languages, region, locale, NTP configuration and automatic-time preferences
remain untouched. The existing macOS time service continues to own ongoing
synchronization, so this is a one-shot correction rather than a new time daemon.

## Constraints and failure behavior

Rows are match, mismatch, unknown or observed without comparison. Summary counts
include only rows with a selected target. All selected comparisons must match for
an overall match; unknown evidence stays incomplete. Regional-format overrides
and other inventory rows remain explicitly uncompared.
A successful command means collection completed; automation must inspect
`summary.status` to decide whether targets match.

Ordinary macOS users may not read automatic-time state through `systemsetup`.
Missing permissions, missing or ambiguous NTP configuration, command failure,
DNS failure, timeout and invalid replies remain unknown. Plain checks never
request privileges; read-only time-settings inspection and clock correction each
require their own explicit action and macOS authorization.
Configured server, running service, kernel state and
reference-time sample are distinct observations. Special-use DNS addresses cannot establish the actual
server's location. Other applications, browser languages, per-app overrides,
other user sessions and the daemon's selected peer remain outside verified scope.
Other operating systems return unsupported.

The design borrows configurable expected values and separate time configuration
checks from [mSCP](https://github.com/usnistgov/macos_security). Its MDM enforcement
checks cannot establish a locally chosen server's current synchronization state.
The single-request measurement follows
[RFC 5905](https://www.rfc-editor.org/rfc/rfc5905.html); it is not a clock discipline
or a replacement time daemon.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --lib native_environment
cargo test --manifest-path engine-rs/Cargo.toml --test native_environment
cargo test --manifest-path engine-rs/Cargo.toml --test native_environment_profiles
cargo test --manifest-path app/src-tauri/Cargo.toml environment_
pnpm --dir app test:unit run src/test/EnvironmentPage.test.tsx src/test/AppNavigation.test.tsx src/test/UiCopyGuard.test.ts --maxWorkers=1
```

Fixtures cover locale precedence, process overrides, ordered languages, absent
evidence, input rejection, kernel sync/read-error decoding, NTP request identity,
timestamp eras and timeout. Correction fixtures verify the no-write gate,
cancellation, failure and independent post-correction measurements.
Integration checks collection persistence, concurrent-edit rejection, deletion,
no HOME/data creation on reads or invalid requests, and no implicit network query.
Frontend tests cover collection CRUD/reopen, target invalidation, explicit prefill,
opt-in correction, destination disclosure, unknown evidence, malformed results,
retry and English rendering.
Completion-feedback regressions cover mixed differences/unknowns, matching
targets with untargeted unknowns, inventory-only results, failed request/retry,
stale notification dismissal and unconfirmed correction despite a matching sample.
Browser mock acceptance at 1000 and 760 px proves the UI; real CLI and isolated
native desktop readback separately establish system and bridge behavior.
