# Remaining Work

Continue with **R7-R9: installed clients, receiving applications and signed updating**.
Source through `45d44a3b` passed complete
[CI](https://github.com/ly87ing/agent-manager/actions/runs/39983028071) and
[GUI CI](https://github.com/ly87ing/agent-manager/actions/runs/39983028252),
including six native engine targets and six installer targets.
Native source/package evidence does not establish full production acceptance.
Local macOS x86_64 source `fac8ed83` was built, installed in Applications and run;
the Launch all control and settings paths were read back from the installed app.
See the [installation receipt](goal-rust-gui.md#2026-10-23-local-macos-installation-and-runtime-readback).

Snapshot: 2026-10-23. Historical baseline: `3ddd9b43`; its final main
[CI](https://github.com/ly87ing/agent-manager/actions/runs/39026462902) and
[GUI CI](https://github.com/ly87ing/agent-manager/actions/runs/39026462915) passed.
The lead reviewer owns implementation and acceptance.
The repository owner owns release authorization in R9.

## Work queue

| ID | Phase | Work and current state | Completion criterion |
| --- | --- | --- | --- |
| R1 | Native tests passed | `156849bc` passed native macOS/Windows/Linux engine jobs; installed-client evidence remains separate. | Retain conflicts; verify current, entries, backups and partial recovery on each native platform. |
| R2 | Native tests passed | Origin/commit, receipt, entries and retained retries passed all three native jobs, including actual uv/Git. | Keep fresh gates before completion/selection and reuse. |
| R3 | Native tests passed | Standalone linking verifies current identity/commands before preference or pointer writes on all native platforms. | Preserve valid controls and unchanged files on refusal. |
| R4 | Then | Implement KDE language and regional-format repair. | Preview scope; preserve settings; reject stale evidence; read back natively. |
| R5 | Then | Implement Linux automatic-time switching beyond matching no-ops. | Verify provider, boot effects, cancellation and partial failure. |
| R6 | Then | Extend GNOME beyond one primary language with explicit formats. | Verify fallback order/inherited formats without PAM/shell edits or logout. |
| R7 | Partial | macOS x86_64 local install/runtime readback and isolated launch/Git cases passed. Installed cross-platform and actual consumers remain open. | Verify UI failures, restart/restore, login/share links and extension loading. |
| R8 | Acceptance | Verify settings in receiving sessions/apps and clock behavior. | Read actual effects independently, including cancellation/failure cases. |
| R9 | Release | Verify signed installed-app updating after separate authorization. | Verify signature, download, install, restart and exact build identity. |
| R10 | Current | Complete source CI and six-target native receipts through `45d44a3b` recorded; installed acceptance remains separate. | Refresh after each new acceptance milestone. |
| R11 | Portability | Resolve skill snapshot support decisions and native evidence gaps. | Verify filesystem/volume limits and explicit refusal of unsupported shapes. |
| R12 | Investigation | Diagnose earlier intermittent macOS DMG busy-eject failures. | Reproduce and verify the cause; a passing retry alone cannot close it. |
| R13 | Native tests passed | TLS/ZIP corrections, GLib backport, optimized Linux iterator and extracted bundle notices passed. Maintenance notices remain recorded. | Maintain source provenance and the [security review](subsystems/dependency-security.md). |
| R14 | Native tests passed | `45d44a3b` passed all six engine and installer targets, including Windows/Linux ARM64 and helper architecture gates. ARM64 release/update routing is prepared; signed and installed-desktop acceptance remains open. | Retain native evidence, payload/target binding and explicit desktop/signed-update limits. |

R4-R6 expand currently unavailable controls; preserve those explicit capability
limits while completing acceptance of the supported operations.

<details>
<summary>Acceptance cases, source entry points and verified baseline</summary>

### R1-R3: program safety

- Start in [`apply.rs`](../engine-rs/src/native_programs/apply.rs), especially
  `execute`, `Step::Move`, `Step::SetCurrent`, the ordinary error path and restore
  planning. The npm/PyPI typed verification refusals already retain candidates;
  preserve that behavior while fixing ordinary recovery.
- For R1, reproduce destination replacement during failure, failed rename/remove,
  missing backup, stale preview and a later failure after selecting `current`.
  Compare selected/unselected files, current pointers and backup bytes before
  and after. Keep an ordinary successful-recovery control.
- For R2, inspect actual Git installation metadata, including commit evidence,
  rather than trusting the requested directory name or installer exit status.
  Cover wrong/unknown source, commit mismatch, missing entries, incomplete retry,
  completed reuse and switch. Retain uncertain candidates without automatic
  deletion. See [Programs](subsystems/programs.md).
- R3's old source accepted a changed npm package during standalone Link on macOS.
  Portable native cases now cover changed npm/PyPI/Git identity, missing commands,
  preserved current/files/preferences and valid linking. Native Windows execution
  established its forwarding-file boundary; a file alone cannot prove its target.

### R4-R6: Linux capability gaps

- Read [Environment checks](subsystems/environment-checks.md),
  [`repair_linux.rs`](../engine-rs/src/native_environment/repair_linux.rs),
  [`linux_kde.rs`](../engine-rs/src/native_environment/linux_kde.rs) and
  [`repair_linux_language.rs`](../engine-rs/src/native_environment/repair_linux_language.rs).
- KDE acceptance needs actual KConfig behavior, immutable/default priority,
  independent formats, unrelated preference preservation and available Qt 5/6
  evidence. Keep unsupported combinations explicitly unavailable.
- Automatic-time acceptance must distinguish active state, provider boot
  enablement, persistence and actual synchronization. Do not infer those from
  timedated's `NTP` Boolean or temporarily manage other providers to pass a check.
- GNOME's preserved raw `Languages` tail is metadata, not proof of ordered UI
  fallback behavior. Existing `LANGUAGE`, `LC_MESSAGES` and `LC_ALL` overrides
  must stay visible. Verify new-session effects separately from setter readback.

### R7-R9: native installation and consumer acceptance

- Use exact artifact SHA256/build identity and a disposable OS user or VM,
  isolated HOME/data and test-assistant paths. HOME isolation alone does not
  isolate OS preferences, clock writes or native login entries.
- Exercise empty/populated/error states and dialogs at about 1000 px and 760 px
  in English and Simplified Chinese. Read actual selected and unselected files
  after UI actions, restart and selective restore. Verify stale refusal and
  preservation of external changes. Do not use retired account operations.
- Prove that the actual assistant loads an applied MCP entry, rule and skill;
  the app's own protocol probe or CLI readback is insufficient.
- Verify new-session language/formats, application-received launch variables,
  GNOME automatic-timezone effects and actual login startup. In a disposable
  system, exercise an excessive clock offset, genuine authorization/cancellation,
  independent remeasurement and audit readback. Private bus/auth fixtures do not
  establish real desktop dialogs, PAM or target-application adoption.
- Follow [Desktop startup](subsystems/desktop-startup.md),
  [Launch configurations](subsystems/launch-configurations.md) and
  [Updater manifest](subsystems/updater-manifest.md). For R9, use two identified
  signed builds and verify each supported installed update format, including
  DEB/AppImage. Tags, Releases and installer publication require separate consent.

### R10-R12: receipts, portability and packaging

- Update [GUI Platform Status](GUI-PLATFORM-STATUS.md) and
  [implementation history](goal-rust-gui.md) with the npm source commit
  `768fb1b7`, final native results and main CI links. Preserve historical failures
  and qualify retries. Local Tauri 27, frontend 706/61 files, script 23 and the
  complete engine suite with 309 macOS library cases passed for this baseline.
- Check [Skills](subsystems/skills.md) before extending snapshot support.
  Remaining boundaries include plaintext chains/non-sibling directory aliases,
  nested gitlinks/index shapes, Unicode/case/filesystem behavior and actual
  multi-volume low-space acceptance. Preserve schema identity, full fork bytes,
  resource budgets and explicit refusal when proof is unavailable.
- R12 follows earlier `hdiutil` busy-eject failures, including the first
  `d241793d` GUI CI attempt. Preserve mount/process diagnostics and use owned
  disposable images. Do not treat retries or broader host cleanup as a fix.

### Verified baseline

- `7d4fde7a` contains the managed npm fix and consolidated branch histories.
  All previous branch tips are reachable from it; local and remote branch
  cleanup left only `main`. No new Release or tag was created.
- The exact commit passed [candidate CI](https://github.com/ly87ing/agent-manager/actions/runs/39025340845)
  and all seven [candidate GUI CI jobs](https://github.com/ly87ing/agent-manager/actions/runs/39025343973),
  including native Windows/Linux/macOS and four installer targets.
- Real npm ordinary/scoped/string/array/direct entry, adoption, version
  resolution and switch/reuse fixtures passed on all three operating systems.
- Main post-push [CI](https://github.com/ly87ing/agent-manager/actions/runs/39025939006)
  and [GUI CI](https://github.com/ly87ing/agent-manager/actions/runs/39025939028)
  were still running at this snapshot. Re-read them before closing R10.

</details>

## Resume and close a task

1. Confirm `git status`, local/remote main identities and the linked CI results.
   Read [AGENTS.md](../AGENTS.md), [lessons](LESSONS.md), the owning
   [subsystem reference](subsystems/README.md), history, callers and nearby tests.
2. Reproduce one selected failure in isolated paths. State scope and done checks
   before editing. For recovery changes, establish how to preserve actual files
   when reverting source cannot undo a completed write. Stop on unknown ownership
   or changed evidence; retain the candidate/backup and report its location.
3. Run the smallest relevant check, then the corresponding full checks:

   ```sh
   cargo test --manifest-path engine-rs/Cargo.toml
   cargo test --manifest-path app/src-tauri/Cargo.toml
   pnpm --dir app test:unit run
   pnpm --dir app build
   ```

   Use pnpm 10, matching CI. Native cases must execute; zero tests, mocks and
   cross-compilation cannot close native acceptance. State skipped checks.
   Documentation-only changes need link/command/scope checks.
4. Record commit, OS/architecture, artifact digest, cases, actual readback and
   limits. Update this queue and the owning reference/history. Push verified
   changes to `main` and read back its SHA. Keep only `main` after temporary
   branch use. Preserve unrelated/user files; never reset/clean in bulk.
