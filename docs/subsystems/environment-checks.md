# Environment checks

## Responsibilities and sources of truth

`environment:profiles` reports `checks_supported` from the native target platform.
The page reads this capability before inspecting the machine. macOS, Windows and
Linux support inspection and comparison. A missing or invalid capability blocks
inspection until the profile read succeeds.

The Environment page inspects regional settings and the caller's process
environment. Targets are optional and grouped into user-created named profiles.
Blank targets provide observations without a comparison. Los Angeles and
Singapore built-in profiles are explicit recommendations, never selected or
applied automatically. No region is inferred from the machine or network.

The desktop and CLI share `environment:check` in
[native_environment](../../engine-rs/src/native_environment/mod.rs). It returns
before secret-source loading, database access, locks or operation logs. Checks
never change system time, preferences, shell files or assistant resources.

On macOS, sources are `/etc/localtime`, the current user's `AppleLanguages` and `AppleLocale`
through Foundation preferences, selected regional-format preferences,
`/etc/ntp.conf`, `systemsetup -getusingnetworktime`, the running `timed` service,
and the read-only Darwin `ntp_gettime` kernel interface.
Only `TZ`, `LANG`, `LC_ALL` and the six common POSIX locale categories are read from
the caller's environment. System local time is read without a process `TZ`
override. Every observation names its source.

On Windows, bounded, profile-free Windows PowerShell reads `Get-TimeZone`,
`Get-WinUserLanguageList`, `Get-Culture`, selected current-user
format values and W32Time configuration/service state. System ICU maps IANA names
to Windows zone IDs; names mapped to the same Windows zone compare equally.
Domain time mode never presents a manual peer as the active configured server.
Time policy ownership or unreadable policy state leaves local switch/peer evidence
unknown; disabled NtpClient or W32Time service settings are not reported as on.
W32Time synchronization and live peer identity remain unknown when unavailable;
service state is not synchronization evidence. POSIX locale variables are inputs
for compatible applications, not proof of a native Windows application's locale.
An explicit Windows `TZ` leaves effective process time zone unknown.
The reader disables PowerShell's module-analysis cache and searches only the
system PowerShell module directory; caller-provided module paths are not executed.
The script reapplies that directory after PowerShell startup, which otherwise
reconstructs the module path and can prepend AllUsers modules. The isolated
policy child uses the same post-startup restriction.

On Linux, typed timedated D-Bus properties supply time settings without service
activation. Time, system-locale and timesync readers use bounded
`Properties.GetAll` calls with `NoAutoStart`; stopped services retain unknown
dependent evidence instead of being started by inspection or comparison.
GNOME inspection verifies the returned AccountsService object's typed `Uid`
against the original caller and reads its primary `Language`. `Languages` is
installer-interest metadata that may also contain the formats locale, not proof
of UI fallback order. POSIX modifiers with no verified language-tag mapping stay
unknown; their independently known regional locale is preserved. Other desktops
do not inherit GNOME account defaults. GNOME's
`org.gnome.system.locale region` supplies the user region, with an empty setting
inheriting the known current-user language. GNOME's automatic-time-zone preference
is a requested switch, not proof of location behavior. Missing user evidence stays
unknown. systemd-localed's D-Bus `Locale` supplies untargeted system-default inventory
only outside KDE. On KDE, the [native reader](../../engine-rs/src/native_environment/linux_kde.rs)
uses `kreadconfig6` or `kreadconfig5` to read
saved Plasma language order, base locale and `LC_*` format overrides. It retains
KConfig's global/cascaded defaults and immutable policy; `--file` is deliberately
avoided because it selects `NoGlobals`. Qt's fixed `plasma-locale` application
name selects `plasma-localerc` in [FullConfig](https://github.com/KDE/kconfig/blob/v6.7.0/src/core/kconfig.cpp), matching Plasma's
[startup reader](https://github.com/KDE/plasma-workspace/blob/Plasma/6.2/startkde/startplasma.cpp).
Both complete reads share an eight-second deadline and must agree. The child
keeps the caller's locale inputs, disables KConfig's writable-home probe and
runs with filesystem data-write restrictions. Missing tool/kernel support or
unstable evidence stays unknown. Empty preferences do not inherit the checker
process or GNOME account defaults. Unmapped language modifiers retain unknown
language order while independently known regional formats remain visible.
These are saved preferences, not proof of installed translations or adoption by
the current desktop or a future login. Inspection never runs Plasma startup,
generates locales or writes configuration. The disposable
[native fixture](../../scripts/verify-linux-kde-preferences.mjs) checks global
immutable/default inheritance, saved language order, independent formats and
absent HOME/config/data without writes; its GUI CI step installs the actual KDE
tools. `/etc/localtime` supplies a zone fallback and `/etc/os-release` supplies
the OS version. Configured timesyncd servers use a typed D-Bus `GetAll` call with
service activation disabled, including on systemd 249. A stopped service remains
stopped and its server lists remain unknown; `show-timesync` and older `busctl
get-property` paths must not be used for this read. A nonempty runtime list
supersedes the loaded system list; link servers still participate. A single
valid candidate from that effective list supplies `ntp.server`; multiple,
invalid or absent candidates remain unknown. Loaded system inventory is not
proof of the current on-disk configuration, and candidate configuration does not
establish the actual peer or synchronization. This follows timesyncd's
[server selection](https://github.com/systemd/systemd/blob/v255/src/timesync/timesyncd-manager.c).
Missing systemd services/properties remain unknown. No daemon or
locale package is installed during a read. The NTP measurement client is shared
on all three platforms. GNOME inspection and inherited formats share one bounded
primary-locale reader: it requires an existing AccountsService unique owner,
checks the current UID before and after reading `Language`, and rechecks bus and
owner identity. Calls carry `NoAutoStart`; a missing owner stays unknown.
The native CLI/private-bus fixture checks runtime precedence, retained link
candidates, empty lists and unknown peer evidence without changing settings.
System settings open Windows Settings, GNOME Settings or
KDE System Settings using fixed destinations; requesting a pane is not readback.
Only an explicit KDE open-settings action runs a bounded `systemsettings --list`
discovery. It opens the installed `kcm_clock` or merged `kcm_regionandlang`;
older installations with both `kcm_translations` and `kcm_formats` open those
two modules together through `kcmshell5`. Missing or unreadable modules produce
an actionable failure. Descriptions and version strings never become arguments.
This discovery creates a Qt application and may use desktop caches; it never
runs during environment inspection, comparison or repair preview.
GUI CI's [KDE fixture](../../scripts/verify-linux-settings.sh) uses real installed
module catalogs and verifies the requested native window's process and module
arguments on its Linux runners. Its HOME/XDG environment precedes private D-Bus
activation; cleanup binds processes to the fixture's CLI session, UID, executable
and start identity. This proves entry-point dispatch and window creation, not
module content, a saved preference or full desktop-session behavior.
GSettings `get` and `writable` run in a child with a Landlock ABI 3 or newer
filesystem restriction (Linux 6.2 or newer with Landlock enabled). This blocks
the verified GLib keyfile/dconf paths for file creation, content writes,
truncation, links and deletion while retaining the original backend, HOME and
policy. It is not a general metadata/IPC sandbox: existing writable file
descriptors, chmod/chown and other unhandled operations are outside this boundary.
The child inherits no real writable output files; the caller and confirmed
setters do not enter its restriction domain. A missing/disabled kernel capability
returns unknown, never an unguarded retry. A missing keyfile directory can make
`writable` false; this is unavailable write evidence, not proof of a policy lock.
The native `GetUserDefaultGeoName` API reads the Windows current-user home region
independently of culture. Automatic settings repair has native adapters on all
three platforms; Linux capabilities depend on the inspected desktop and provider,
with unavailable controls listed explicitly in the preview.
Explicit clock correction is implemented on macOS, Windows and Linux; platform acceptance is
tracked separately from compilation and source tests.

Language comparison uses the complete ordered list, ignoring tag casing.
Repair planning and independent readback use that same rule, so OS tag
canonicalization does not request repeated writes or report a false failure.
List order, unrelated values and the raw preview baseline remain exact.
Region uses the Windows home region or the country component of the macOS/Linux
user locale. Each process locale category
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

Opening Environment reads local observations and saved profiles without any
network query or write. Selecting a profile clears the old report. Compare
profile compares its values and queries its explicit time server,
when configured. The destination is disclosed before checking. Comparisons never
correct time, including profiles saved with the legacy `auto_sync` flag. No
profile is selected by default.

The Environment profiles tab maintains a separate list with create, edit,
duplicate and delete actions. The Compare and apply tab selects a profile for an
operation. Saving a definition does not select it, run a comparison or apply it.
Editing or deleting the selected definition invalidates its comparison; other
definitions do not change the current selection or report. Profile-list reads
are independent of environment-read failures. Blank clock tolerance is valid
unless a clock-correction action is requested. Missing names, empty criteria and
invalid tolerances are explained in the editor's fixed footer.

`environment:profiles` returns saved definitions in `items` and read-only
recommendations in `builtins`. Both desktop and CLI receive the same built-in
definitions from Rust. Reading does not persist them or create HOME/data. Their
`builtin:` IDs cannot be saved, deleted or restored as database records through
the existing saved-ID validation. Copying creates an ordinary new custom profile;
later edits to that copy do not change the built-in. Backups contain only saved
custom definitions.

| Built-in | Time zone | Exact language list | Region | Process locale | Reference server |
| --- | --- | --- | --- | --- | --- |
| Los Angeles (US) | `America/Los_Angeles` | `en-US` | `US` | `en_US.UTF-8` | `time.nist.gov` |
| Singapore (SG) | `Asia/Singapore` | `en-SG` | `SG` | `en_US.UTF-8` | `nets.org.sg` |

Both require strict raw-locale consistency, automatic time on, automatic time
zone off, kernel synchronization and a 2000 ms clock tolerance. Unset raw locale
variables may inherit; explicitly set nonempty variables must agree. The exact
language list is intentional, so adding another language is a mismatch until
the user explicitly copies and changes the standard. These are bounded product
criteria, not a claim that every user in those regions has identical preferences.
Singapore uses the supported macOS POSIX English locale `en_US.UTF-8` for process
compatibility. Foundation supports the Singapore system locale independently;
the tested macOS POSIX locale inventory lacks `en_SG.UTF-8`, and requesting it
with `/usr/bin/locale` fell back to `C`. This profile therefore does not assert
Singapore-specific POSIX monetary/date formats. The system region target remains
`SG`; the explicit process target is visible and can be changed in a custom copy.

The [NIST server list](https://tf.nist.gov/tf-cgi/servers.cgi) recommends the
load-balanced `time.nist.gov` name. Its published current nodes are in Colorado
and Maryland, not Los Angeles. Singapore's
[A*STAR NMC time laboratory](https://www.a-star.edu.sg/nmc/Research-and-Development/Labs/FL)
publishes `nets.org.sg` as a free Stratum-1 service for UTC(SG). Authority and
regional suitability are the selection basis, not an unmeasured claim of best
latency or guaranteed availability. Each explicit comparison queries only its
selected server and reports unreachable or invalid evidence as unknown. There
is no silent fallback, automatic server ranking or geographically inferred
switch. The provider and official-reference URL accompany each definition so
users can review the source or copy the profile to choose another server.

Apply profile opens an action preview with the profile name, relevant expected
values, available observations and the effect's scope. Nothing executes before
the action-specific confirmation. The supported targets are:

| Target | Effect |
| --- | --- |
| System clock | Measure, correct only a clearly excessive offset with system authorization, then independently verify. Requires an explicit server and tolerance. |
| Application launch environment | Prefill an unsaved launch recipe with the profile's time zone and locale; the user chooses an application and explicitly saves or launches it. |
| Date/time or language/region settings | Open the corresponding system pane for manual adjustment; opening it does not establish that settings were applied. |

### Automatic repair

#### Windows

Windows repair uses the same preview/confirmation, stale-base rejection, per-user
mutation lock, audit and independent readback as macOS. Its preview includes the
native context and a platform field. System-only setters request UAC, reject an
expired boot-time lease or changed system fingerprint before each write, and
stop on uncertain outcomes. User APIs and HKCU always run in the original caller,
even when UAC uses a different administrator. Domain W32Time modes, managed time
policies and unknown evidence are unavailable; the tool never overrides policy or
changes privacy. Native policy calls run in a separate profile-free process with
a two-second deadline. A blocked policy API leaves only the dependent policy
evidence unknown; the remaining system snapshot stays readable. Fresh privileged
snapshots repeat this bounded read and never treat a timeout as unmanaged.
Repair and clock correction resolve the mutation lock's user SID directly from
the current process token through read-only Win32 APIs; acquiring the lock does
not launch PowerShell. The existing `Global` mutex name and canonical SID notation
retain exclusion with older binaries and across sessions/data directories.
Native coverage compares an independent `whoami` SID and an old-format mutex,
then checks a separate holder/contender process pair, release and reacquisition.
Time-zone policy uses the public Windows
[NamedPolicy API](https://learn.microsoft.com/en-us/uwp/api/windows.management.policies.namedpolicy).
Enabling automatic time zone also requires known permitted machine location
settings; disabling does not require location permission.

Windows region uses [SetUserGeoName](https://learn.microsoft.com/en-us/windows/win32/api/winnls/nf-winnls-setusergeoname), preserving culture and regional formats.
Language reordering retains existing language objects and their input options.
New languages use OS defaults. A required format-independence policy change is
listed separately in the preview; unreadable languages never change that policy.
Readback must retain the original culture/formats and format-independence protection.
Replacing a single manual NTP peer retains its existing polling/client flags;
unknown flags or multiple peers are unavailable.

Windows has no launchd-style temporary login environment. `TZ`, `LANG`, `LC_ALL`
and the six locale categories are unavailable by default. The separate
`--allow-persistent-user-env` choice must appear in both preview and confirmation;
it is bound into the preview fingerprint. The desktop checkbox is off by default
and requests a new preview. These user variables persist after logout and affect
future logins and new Explorer-launched processes. Windows sends
[`WM_SETTINGCHANGE`](https://learn.microsoft.com/en-us/dotnet/api/system.environment.setenvironmentvariable?view=netframework-4.8.1); other applications may refresh their environment. The checker
process is unchanged. No shell files or login
entries are edited.

#### Linux

Linux repairs selected regional variables in the existing systemd user manager.
It never creates or starts that manager or a user
bus. The bus ID, manager owner and original caller UID are bound for the entire
operation; a manager or bus replacement stops the repair. Reads retain only the
selected regional keys, including known absence, and setters recheck their
baseline before each direct D-Bus call. Independent reads verify every result.
The locale must be installed and work in an isolated locale subprocess.

These values affect subsequent user units and applications started by that
manager. They are not proof of desktop-wide inheritance; the caller and existing
applications keep their environment. No shell files or persistent login defaults
are edited. Preview, stale refusal and confirmed no-op do not write HOME or the
database. Linux clock correction remains independent.

System time-zone repair uses the original caller's exact Polkit
`org.freedesktop.timedate1.set-timezone` action with PID, start time and UID.
Read-only time-zone snapshots, including repair preview and independent readback,
forbid D-Bus activation; a stopped timedated service leaves that evidence unavailable.
Authorization finishes before a fresh comparison of timedated and disk evidence;
the subsequent native Rust D-Bus setter uses the same process that owns the
authorization, with both payload and message flags noninteractive. The original caller
independently checks both sources. No root helper is used for this action.
Connection setup and dispatch share an eight-second bound; transport uncertainty
remains an unknown outcome and stops subsequent changes. A timeout cannot revoke
a request already dispatched to the system service.
The current slice requires a UTC hardware clock and a root-owned zoneinfo symlink
matching timedated. Unknown/local RTC and cache/disk disagreement remain
unavailable. This preserves the hardware clock.

On a GNOME session, automatic-time-zone preferences use the original user's
GSettings backend and remain user-scoped in the preview. They join the ordered
time batch: authorization for an actual system timezone change finishes before
any preference write, then disable precedes fixed timezone, and enable follows
it. Auto-only changes do not request system authorization. Every step compares
fresh related evidence and independently reads back its result; only that exact
own change advances the baseline. Unknown/writable-policy loss, bus/daemon
replacement, changed authorization evidence or outcome uncertainty stops later
items, including session variables. Fixed-zone changes while automatic timezone
remains enabled are unavailable; there is no hidden temporary pause or retry.

Enabling requires location already enabled, an existing caller-owned GNOME
Datetime D-Bus service, and a noninteractive authorization check for that service's
actual PID/start-time/UID. Disabling does not require location or daemon
permission. No location permission, GeoClue client, daemon lifecycle, coordinates
or user privacy settings are changed. Preference readback does not prove that
GNOME has drained an earlier request or that geolocation/automatic switching
works. A final timezone drift is partial, not an invitation to compete with the
daemon. CLI `verification_limits` and the dialog retain that distinction;
auto-only changes do not request an application restart.
Automatic time-service activation/persistence and desktop language repair remain
unavailable here. A known timedated `NTP` value matching the selected
`automatic_time` target is an unchanged item, so an otherwise empty confirmed
repair is a zero-write no-op. A different or unknown value remains unavailable;
the preview binds the observation so subsequent drift refuses the old preview.
This Boolean reports the native active-service switch, not provider boot
enablement or synchronization. Results retain that limit. `SetNTP` is not used:
its cached provider list and installation effects are not exposed by this
Boolean, and its force-enable/disable operations can reach other units and
aliases. Merely checking the Boolean before and after would miss those changes.
GNOME regional-format repair is described below.

GUI CI verifies actual apply, future-unit inheritance, no-op, stale refusal and
restoration on a disposable runner's existing user manager, with isolated HOME
and data. Its separate timezone receipt exercises an actual caller-authorized
timedate1 write, disk readback, no-op, stale refusal and cache/disk disagreement.
A temporary, exact-user/exact-action Polkit fixture is exclusively created and
removed. A second exact-process AUTH_SELF_KEEP case uses a CI-only root
authentication agent to exercise the actual daemon's challenge/cache/setter
protocol, including an external settings change during authorization. That agent
substitutes the credential verifier; it does not establish PAM, password or
desktop authentication-dialog acceptance. Cookies are never logged or persisted.
If the hosted image has globally writable zoneinfo, the fixture first verifies
the production refusal. It temporarily removes group/world write bits from its
exact test files, binding root ownership, inode and content, and restores their
original modes independently during cleanup. Production trust is unchanged.
An additional real GSettings/keyfile fixture isolates the user's configuration
directory and exclusively owns a disposable Datetime bus name. It checks ordered
repair, denial before user writes, privacy/service-policy gates, stale identity,
no-op and readback. That owned name is a test double, not desktop/geolocation
acceptance. See the [GNOME monitor](https://github.com/GNOME/gnome-settings-daemon/blob/master/plugins/datetime/gsd-timezone-monitor.c)
and [preference listener](https://github.com/GNOME/gnome-settings-daemon/blob/master/plugins/datetime/gsd-datetime-manager.c).

GNOME single-primary-language repair uses the existing current-UID
AccountsService object's `SetLanguages` method. The current `Language` must match
the first element of a nonempty typed `Languages` array. An installed UTF-8 locale
must map to the exact requested language tag. The raw metadata tail stays in its
original order; it is not treated as an ordered UI fallback list. Multiple target
languages and unknown evidence remain unavailable. Existing explicit formats stay
unchanged by the language setter. Inherited/empty formats require a simultaneously
selected target country, coherent primary-language identity and an installed,
writable formats locale for that country. A visible `language.independent_formats`
dependency saves that full formats locale before changing language, including when
the effective country already matches. The normal region row still shows the full
before/expected locale. Language-only profiles never freeze inherited formats.

The preview binds the native primary locale, raw array, bus/owner/object/UID,
selected installed locale and the original formats/backend context. Confirmation
uses the existing same-process Polkit preauthorization, followed by a fresh full
baseline check. The setter addresses the bound unique owner with `NoAutoStart`
and no interactive-authorization flag. Its own authorization still applies;
preauthorization does not replace the service's decision. Cancellation, refusal,
changed evidence, an unknown reply or mismatched readback stops later writes.
All authorizations complete before the formats dependency writes. Its complete
native readback advances only the formats baseline; subsequent language readback
advances only language evidence. A failed language step retains independently
verified saved formats in both result rows, requests user-controlled relogin and
does not automatically restore them. An external formats reset or unreadable
readback invalidates both formats results. Repeating a completed repair performs
no dependency write.
The API has no compare-and-swap guarantee against an uncoordinated external writer.

Verification covers the existing service's primary preference only. AccountsService
does not propagate every cache-file save error to its reply. Durable storage,
current desktop adoption and a future login therefore remain unverified. Existing
`LANGUAGE`, `LC_MESSAGES` or `LC_ALL` overrides may still affect displayed language.
The tool does not rewrite PAM/shell files, install translations, log out or restart
applications. See the upstream [setter](https://sources.debian.org/src/accountsservice/23.13.9-8/src/user.c/),
[authorization](https://sources.debian.org/src/accountsservice/23.13.9-8/src/daemon.c/)
and [GNOME consumer](https://sources.debian.org/src/gnome-control-center/1%3A48.4-1~deb13u1/panels/system/region/cc-region-page.c/).
Native Linux acceptance is required before promoting a change to this writer; the
private provider fixture uses the distro AccountsService and Polkit daemons; the
existing CI authentication agent exercises cancellation and authorization drift.
The inherited-formats cases cover selected changed/matching countries and zero
formats writes on cancellation or authorization drift. The separate lost-reply
fixture covers retained partial formats, service reply loss and an external reset.
That fixture does not establish installed-desktop adoption.

GNOME regional-format repair writes only the original user's
`org.gnome.system.locale region` through GSettings. The preview shows the full
effective locale; an empty preference inherits the independently known current
primary locale. The selected country replaces only that locale's country,
preserving language, script, codeset and modifiers. The resulting locale must be
installed and load successfully; the tool never generates locales or substitutes
another language. Unknown inheritance, an unreadable/locked backend and other
desktops remain unavailable. The preview binds the raw inheritance setting,
effective locale, writable evidence, caller UID and backend location inputs.
Inherited formats read only the existing AccountsService owner's typed current-UID
primary locale, binding its bus/owner/object identity and rechecking the raw
preference. Caller LANG and installer-interest metadata are not fallbacks.
The ordered Linux batch retains its original full context before each write,
including user-scoped formats; a same-effective-value inheritance/source change
still stops. Fresh reread and complete State readback surround the confirmed
setter; verification requires the explicit raw locale and original source/UID,
not merely an effective value inherited after an external reset. Untargeted
preferences and UI language are preserved. With available evidence, a matching
preference is a zero-write no-op; a changed preview stops before audit/database
initialization. A change after that initial validation may retain the requested
audit but still stops before the regional write.
The result records preference-only verification and asks the user to save work
and sign out/back in themselves. It never logs out, restarts applications or
claims that the current desktop or a later login has adopted the formats. A native
isolated keyfile fixture exercises real preview/confirmation, stale refusal,
untargeted preference preservation, idempotency and unavailable locales/desktops.
No personal host settings are test targets. Both scripts are guarded
against execution outside that explicit CI scope.

Linux manual NTP repair has two separate targets: the original running
timesyncd instance's `RuntimeNTPServers`, and a dedicated file at the standard
host path `/etc/systemd/timesyncd.conf.d/90-agent-manager.conf`. Preview names
both operations, with independent unchanged decisions and directory creation
when needed. An already-matching runtime list does not hide a missing saved file.
The runtime list must be empty or already match the selected server in this
slice. This does not change inspection's effective runtime-or-system plus link
candidate contract, or establish a currently selected NTP peer.

Repair requires a supported existing native interface and a trusted system-bus
socket. It binds bus IDs, unique owners, the PID-1 manager, daemon PID/birth/UID,
canonical loaded-unit identity and invocation, loaded server arrays, poll limits
and unit enablement. `GetUnitByPIDFD` looks up an existing instance without loading
a unit; every native service call uses `NoAutoStart`. Typed introspection verifies
the setter's input signature before any write. Ordinary-user preview does not
read another UID's `/proc/PID/exe`; that read requires ptrace permission. The
elevated helper independently pins the root-owned installed executable's identity,
metadata and digest before READY and requires the same proof before each change.
Unavailable process, API, configuration or metadata evidence stays unavailable.

The fixed-path reader bounds configuration bytes, names and work checkpoints.
It rejects symlinks, hardlinked files, unsafe ancestors, foreign target contents,
unsupported syntax, later-priority overrides and lower same-name files. Existing
admitted `0600`/`0640`/`0644` modes and groups are preserved; private files an
ordinary caller cannot read remain unavailable in preview. The first slice
refuses extended destination/parent metadata instead of dropping ACLs or labels.
Its file-publication slice requires the opened descriptor's mount ID to resolve
to the concrete ext4 driver, as well as the ext-family filesystem magic. Other
or stacked filesystems remain unavailable. `flistxattr` must successfully report
no attributes; unsupported inspection is not an empty result. Both
`FS_IOC_GETFLAGS` and `FS_IOC_FSGETXATTR` must succeed on that descriptor. Only
ext4's extent-layout flag and directory-index flag are admitted; all policy or
unknown bits, nonzero project IDs and allocation hints are refused. The existing
target, creation parent, new directory receipt and staged file use this gate.
Special permission bits, including inherited setgid-directory policy, are refused.
This finite ext4 admission does not claim to attest arbitrary filesystem policy.
A retained anonymous memfd's trusted attribute directly proves the helper's
current initial-namespace inspection privilege. It creates no named file or
persistent setting. The fixed helper never changes thread credentials or
namespaces; its workers inherit those privileges. Unsupported probes fail closed.

After confirmation, `pkexec --disable-internal-agent` launches the existing
GUI-independent helper with a cleared environment. Its fixed NTP entry accepts
only a server, original PID/birth/UID, at most two minutes of boot-time lease and
confirmation. It accepts no HOME, database or file paths. The helper checks the
real parent, `PKEXEC_UID`, caller pidfd and pipe liveness; it takes a cooperative
host lock on the trusted `/etc/systemd` directory. READY carries the fresh common
digest and is zero-write. The caller obtains any timezone authorization and
rechecks its complete original Settings context before COMMIT. NTP completes
before GNOME and session changes; the existing automatic-timezone ordering is
retained within the subsequent timezone batch.

The helper rechecks its lease, instance, executable and exact configuration before
each declared mutation. It writes only the dedicated override using exclusive
staging, file sync, rename and directory sync; new target publication cannot
overwrite a concurrently inserted file. Existing replacement uses a fresh
comparison under the cooperative lock, which does not exclude external editors.
Only durable file publication permits the native runtime setter. Its authorization
fast path requires the retained bus connection's authenticated root UID and the
recognized daemon's non-root real UID. The native setter may query servers,
reselect a peer and adjust the clock before the separate measurement-gated clock
stage; preview explicitly discloses this.

DONE contains narrow receipts, not raw configuration or environment values.
Independent caller readback must match an expected state projected from the
original baseline with only the owned file/directory receipts and runtime list.
ACK releases the helper's host lock. Partial publication, drift, deadline expiry,
lost responses or unsuccessful acknowledgment stop subsequent writes. EOF and
pidfd/deadline checks gate new writes; a non-root group signal cannot prove an
elevated operation or an already-dispatched RPC was revoked. No blind rollback,
retry, service restart/reload, enablement change or provider installation occurs.

Every NTP repair result, including no-op, retains this limit: whether a future
instance loads the saved standard-host-path override on this device remains
unverified. Runtime/file readback does not prove actual peer, synchronization or
clock accuracy. The guarded disposable
[native NTP fixture](../../scripts/verify-linux-ntp.mjs) checks real vendor APIs,
ordinary-user authorization, independent readback and external restart loading;
that runner-specific loading evidence does not remove the user's limit. Native
dialog/PAM and installed-desktop acceptance remain separate.

#### macOS

**Repair automatically** is a separate action for the explicitly selected profile.
`environment:repair-preview --spec JSON` reads the current settings and returns
the targeted rows, original values, proposed values, scope and `base` fingerprint.
It performs no authorization, network query, database write or system change.
The fingerprint binds the target, stable setting values and the calling UID/
launchd context; moving timestamps, clock samples and kernel error estimates are
excluded. No saved profile or region is selected implicitly.

`environment:repair --spec JSON --base SHA256 --confirm` requires that same target
and fingerprint. It re-reads the settings before recording a request or writing,
then checks each baseline again immediately before its change. Stale values stop
further changes and require a new preview. System commands lack atomic CAS, so
this detects observed external changes without guaranteeing exclusion of other
system tools or users. Repair and one-shot clock correction share a private
per-UID advisory file lock independent of the configurable app data directory.

| Target | Write and verification |
| --- | --- |
| System time zone | `systemsetup -settimezone`; read `/etc/localtime` again. Disable targeted automatic time zone first. |
| Automatic time zone | Write only `Active` in `/Library/Preferences/com.apple.timezone.auto`; independently read that preference. This does not verify location-service behavior. |
| NTP server and automatic time | `systemsetup -setnetworktimeserver` / `-setusingnetworktime`; separate read commands verify the result. |
| Ordered languages | Write only current-user `AppleLanguages`; re-read through Foundation. |
| Region | Replace only the country component in current-user `AppleLocale`, preserving language, script, extensions and all separate format overrides; re-read through Foundation. Unknown/unparseable originals are not replaced. |
| Future-process environment | Allowlisted `launchctl setenv` for `TZ`, `LANG`, `LC_ALL` and the six supported locale categories; separate `getenv` readback. |

System changes run in a fixed, clean command environment with macOS administrator
authorization. Values are validated and quoted as data. The privileged batch
re-reads baselines after authorization and reads again after every write; no
password, arbitrary executable or arbitrary preference path is accepted.
Locale repair first verifies the platform inventory and an isolated real
`locale` invocation. Unsupported locales are rejected before any write.
See Apple's [systemsetup reference](https://support.apple.com/guide/remote-desktop/apd95406b8d/mac).

Session repair requires an identified Aqua graphical-login context: manager UID
must match the caller, and its native Mach audit token must identify an existing
`login/ASID` domain. No launchd environment dump is read to identify the domain.
The preview displays and fingerprints that `gui/UID` + `login/ASID` scope.
`managerpid` alone is insufficient because modern launchd always reports PID 1.
Background/SSH or unreadable context identities leave session items unavailable;
read failures remain distinct from successfully read empty values. Session
settings expire at logout. Existing
apps, terminals and this checker retain their environment; shell startup files,
application overrides and explicit launch configurations can still override new
session values. No shell file or login item is created, and no app is restarted.
Readback of launchd values does not prove a target app's effective environment.
The result preserves current-process mismatches and prompts the user to save
work and relaunch affected applications through an appropriate launch entry.

After successful settings repair (or a no-op), a profile with an explicit server
and clock tolerance enters the existing fresh-measurement clock gate, then returns
an independent comparison. Synchronization state, actual background peer identity
and measured clock accuracy are observations, never writable success flags.
Missing evidence remains unknown. The mutation lock spans settings, guarded clock
correction and final verification. Automatic-time criteria receive a new
authorized read after those operations; an earlier batch read is never reused to
override fresh observations. Cancellation/failure of this final read leaves the
switch unknown. A clock/auth failure does not erase completed setting changes.

Each result distinguishes unchanged, verified, verified-after-command-error,
unverified, stale, not applied and unknown outcomes. Errors stop later changes;
commands are not retried and completed changes are not automatically undone.
Preview cancellation writes nothing. Once confirmed, request/outcome audits
contain setting IDs and statuses only; cancellation of administrator authorization
can retain that request without performing OS writes. Failure to record the
request stops before authorization. Completion-audit failure retains the actual
results and shows a warning. A source rollback does not restore OS settings:
review the original values in the preview/results before explicitly restoring
them or applying a previous profile, accounting for intervening external changes.

```sh
cargo run --manifest-path engine-rs/Cargo.toml -- environment:repair-preview --spec '{"region":"US"}' --json
# Review the returned rows, then supply their exact base fingerprint:
cargo run --manifest-path engine-rs/Cargo.toml -- environment:repair --spec '{"region":"US"}' --base YOUR_PREVIEW_SHA256 --confirm --json
```

The application target is unknown during launch prefill; Agent Manager's observed
process environment is not presented as evidence for that application. Saved
launch recipes retain their explicit values rather than a live profile binding.
Profile edits do not change existing recipes or running applications. Clock
request failures keep the preview and previous comparison available for retry.

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
the name and target in `environment_profiles` inside the existing
`native-manager.db`, with `auto_sync:false` for the existing storage contract.
The old flag remains readable and portable in backups but is not an execution
instruction. Saving does not apply settings. Missing database
or table reads as an empty list without creating either. Save/delete use an
Immediate transaction and revision-based compare-and-swap, preserving concurrent
edits. Duplicate names are rejected. No separate settings file or account state
is involved. The collection endpoints are `environment:profiles`,
`environment:save-profile --spec JSON [--id ID --base SHA256]` and
`environment:delete-profile --id ID --base SHA256`.

[Configuration backups](extension-backups.md) include these collections as an
optional selectable module. Restore validates targets and unique collection
names without inspecting or applying system settings. Restoring `auto_sync:true`
does not query or correct time; correction still requires a later explicit apply
confirmation.

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

The CLI explicitly adds `--probe-time`. The page's Compare profile
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

`environment:sync-time --spec JSON --confirm` exposes standalone clock correction;
confirmed automatic repair reuses the same guarded implementation.
Its target must explicitly include a valid `ntp_server` and
`max_clock_offset_ms`. Apply profile and clock-remediation actions preview this
operation and require an explicit clock-correction confirmation. Compare profile
never selects this endpoint. Selecting or saving a profile never
corrects time. No background schedule runs.

The endpoint makes a fresh independent measurement. A matching offset returns
`not_needed`; missing or overlapping evidence returns `unavailable`. Neither
case requests privileges. A clear mismatch invokes macOS's `/usr/bin/sntp` once
against the measured IP using a fixed AppleScript and administrator authorization.
The program path, options and clean environment are fixed; the only argument is
a parsed IP quoted as data. Passwords are handled by macOS, never by Agent Manager.
No privileged helper/service is installed and no authorization policy is changed.
See Apple's [administrator authorization guidance](https://developer.apple.com/library/archive/technotes/tn2065/_index.html).

SNTP uses `-S -s -n 1 -g 4000 -t 5`; it may step the clock or request a gradual adjustment.
The authorization subprocess has a 150-second deadline. A measurement
older than ten seconds is rejected inside the authorized command
before SNTP starts; a delayed authorization requires another explicit attempt.
A second independent measurement follows correction, cancellation or failure. Query intervals are at
least four seconds within this flow, including around the privileged request.
Only an applied command with a matching new measurement returns `verified`.
Other outcomes are `unverified`, `cancelled`, `unavailable` or `failed`; an exit code alone
cannot establish success. Results include the pre-correction offset and
uncertainty in `time_repair`.

A clear mismatch records a `requested` operation before administrator
authorization. If that audit cannot be saved, correction stops before requesting
privileges. Completion records the outcome and numeric before/after measurement
evidence under the same run identifier. Server addresses, process variables and
authorization output are excluded. If the completion audit fails, the report
retains the actual correction outcome and sets `time_repair.audit_recorded:false`;
the page shows a warning alongside the new measurement. Matching or uncertain
measurements return without an audit or database creation. Ordinary checks and
authorized read-only inspection remain zero-write.

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
cargo test --manifest-path engine-rs/Cargo.toml --test native_environment_repair
cargo test --manifest-path app/src-tauri/Cargo.toml environment_
pnpm --dir app test:unit run src/test/EnvironmentPage.test.tsx src/test/EnvironmentProfiles.test.tsx src/test/EnvironmentRepair.test.tsx src/test/AppNavigation.test.tsx src/test/UiCopyGuard.test.ts --maxWorkers=1
```

Fixtures cover locale precedence, process overrides, ordered languages, absent
evidence, input rejection, kernel sync/read-error decoding, NTP request identity,
timestamp eras and timeout. Correction fixtures verify the no-write gate,
cancellation, failure and independent post-correction measurements.
Integration checks collection persistence, concurrent-edit rejection, deletion,
no HOME/data creation on reads or invalid requests, and no implicit network query.
Frontend tests cover independent profile CRUD/reopen, target invalidation, explicit prefill,
confirmed correction, legacy-flag read-only comparison, destination disclosure, unknown evidence, malformed results,
retry and English rendering.
Built-in regressions cover read-without-state, immutable IDs, independent copies,
explicit selection without execution, full strict target forwarding and preserved
comparison after reload. Existing backup tests retain the custom-only storage
contract.
Completion-feedback regressions cover mixed differences/unknowns, matching
targets with untargeted unknowns, inventory-only results, failed request/retry,
stale notification dismissal and unconfirmed correction despite a matching sample.
Browser acceptance at 1000 and 760 px exercises CRUD, read-only comparison,
apply preview/cancel and unsaved launch prefill against an isolated real CLI.
Unavailable clock evidence is exercised without correction. Isolated native
desktop save/close and independent CLI readback establish bridge persistence;
these checks do not claim real administrator-authorized clock correction.
Automatic-repair coverage additionally exercises stable preview fingerprints,
no-op convergence, stale/audit failure gates, authorization cancellation, partial
failure and preserved current-process differences. The generated privileged
shell runs against disposable settings for write/readback and stale refusal;
real macOS integration exercises read-only preview and confirmed no-op without
writing system preferences. This does not establish real administrator-authorized
system-setting writes or target-app inheritance; those require a disposable macOS
environment or a separately chosen, authorized live application.

### Windows and Linux explicit clock correction

`environment:sync-time --confirm` uses the shared fresh NTP measurement and
uncertainty gate. Windows requests UAC and Linux requests Polkit through `pkexec`.
The restricted native helper takes a fixed peer address, tolerance, boot-time
deadline and explicit confirmation. After authorization it queries that peer
again; it never replays the offset measured before the authorization dialog.
Expired requests, ambiguous measurements, and samples already within tolerance
perform no write. Immediately before writing it checks the deadline, sample age
and wall-clock movement against monotonic elapsed time. Windows enables its own
clock privilege and calls `SetSystemTime`; Linux uses the kernel-relative
`clock_adjtime` offset operation, avoiding external read/compute/set delays.
Neither changes time-service configuration. Cancellation/failed authorization
stops the write; an independent NTP query establishes the result, never the exit
code alone. A current-user OS mutation lock prevents concurrent repair and clock
correction across different application data directories.

The Tauri build hook includes the GUI-independent native helper in Windows and
Linux resources. Linux executes a private, size-bounded temporary copy under the
root-owned sticky system `/tmp`, rejecting unsafe parents and ignoring `TMPDIR`, so that
authorization also works when an AppImage sidecar resides on a user-only FUSE
mount. The helper's two-minute boot-time lease expires before the parent command's
150-second deadline. Malformed/expired native CLI fixtures stop before privilege
checks, NTP, HOME inspection or database creation. Pure tests cover uncertainty,
already-corrected clocks and clock steps in both directions; they do not prove an
actual privileged clock adjustment or its effect on an installed application.
