# Quota and Tray

Shows how much of each account's allowance is used and when it resets: on the Accounts page, in the tray menu and tray panel, in the ⌘K palette, and in an optional reminder. Everything here reads local files and only displays; nothing switches accounts or writes assistant configuration.

## Responsibilities and sources of truth

| Part | Responsibility | Source |
| --- | --- | --- |
| Reading quota | `read` (main accounts) and `read_profile` (separate accounts) return windows with used percent and reset time, plus `checked_at`. Claude: the newer of the status-line snapshot `<data dir>/usage/<encoded config dir>.json` and `.claude.json` `cachedUsageUtilization`; Codex: the last `rate_limits` in the newest session file, read from its last 1 MiB. Read-only, no network | [`agent_usage.rs`](../../engine-rs/src/agent_usage.rs) |
| Where readings appear | `agents:status` and `profiles:status` carry the reading as `observed.usage`; `buildAccounts` puts it on each account | [`native_agents.rs`](../../engine-rs/src/native_agents.rs), [`native_profiles/status.rs`](../../engine-rs/src/native_profiles/status.rs), [`accounts.ts`](../../app/src/lib/accounts.ts) |
| Display rules | Window labels, reset countdown, expired windows hidden, stale after 24 hours, tightest window, tray summary text, colour bands, sorting by soonest reset or most left | [`agentUsage.ts`](../../app/src/lib/agentUsage.ts), [`QuotaCells.tsx`](../../app/src/components/QuotaCells.tsx) |
| Background refresh | The palette, mounted for the whole app life, rereads `agents:status` and `profiles:status` every 10 minutes even while the window is hidden, and publishes the tray menu | [`CommandPalette.tsx`](../../app/src/components/CommandPalette.tsx), [`queries.ts`](../../app/src/lib/queries.ts) |
| Tray menu | Native menu grouped by assistant: name · launch command · tightest window with countdown. Choosing an account opens the palette at it | [`tray.rs`](../../app/src-tauri/src/tray.rs), [`trayActions.ts`](../../app/src/lib/trayActions.ts) |
| Tray panel | Window labelled `tray-panel`, 380×520, created on first left click, placed by `panel_position`, hidden on blur or Escape. The same page renders `TrayPanel` instead of the app when the window label matches | [`tray.rs`](../../app/src-tauri/src/tray.rs), [`TrayPanel.tsx`](../../app/src/components/TrayPanel.tsx), [`main.tsx`](../../app/src/main.tsx) |
| Quota reminder | `dueReminders` picks windows with less than a tenth of their length left and at most half used, from readings that are not stale; `sendDueReminders` sends each window round once. Opt-in in Settings; on/off and sent keys live in `localStorage` | [`quotaReminder.ts`](../../app/src/lib/quotaReminder.ts), [`SettingsPage.tsx`](../../app/src/pages/SettingsPage.tsx) |

## Runtime path

1. A status query reads each account's files and returns `observed.usage`.
2. The Accounts page renders `QuotaCells` per row and can reorder rows with `sortByQuota`; the palette builds tray rows with `trayUsageSummary` and calls `tray_set_accounts`.
3. Every minute the palette's clock ticks: tray summaries drop windows past their reset, and `sendDueReminders` runs.
4. Left click on the tray icon (macOS, Windows) toggles the panel; right click, and any click on Linux, opens the native menu.

## Constraints and failure behavior

- A reading is only as new as the assistant's own record; past-reset windows are hidden, and readings older than a day are marked as old and never trigger a reminder.
- The tray menu is replaced only when all three status queries succeeded, so a transient failure keeps the previous menu.
- If the panel window cannot be created, a left click shows the main window instead.
- The window-state plugin skips the panel (`with_denylist`), so it is always placed next to the icon.
- Notifications need system permission; when it is refused the Settings switch stays off and says so.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test agent_usage
cargo test --manifest-path app/src-tauri/Cargo.toml tray
cd app && pnpm vitest run src/test/agentUsage.test.ts src/test/quotaReminder.test.ts src/test/TrayPanel.test.tsx src/test/AccountsPage.test.tsx
```

Panel placement, blur, and copy were last accepted on macOS with a separate-identifier debug build (see [LESSONS](../LESSONS.md)); Windows placement and a real system notification have not been observed.
