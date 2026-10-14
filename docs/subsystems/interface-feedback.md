# Interface feedback

## Responsibilities and sources of truth

The interface distinguishes action outcomes, page state, form validation and
persistent reports. Backend envelopes and actual readback determine success;
notification styling never changes command arguments, permissions or stored data.

## Runtime path

All production notification producers use
[notifications.ts](../../app/src/lib/notifications.ts). The wrapper applies close
controls, four-second success/information and ten-second warning/error defaults,
bounded wrapping text and modal-aware placement. Undo actions keep ten seconds;
detailed environment reports may explicitly retain that duration. The themed
renderer shows at most two expanded notifications.

[ExecutionProvider](../../app/src/lib/execution.tsx) owns ordinary write outcomes.
Success descriptions combine resource-specific results, effect conditions and
backend warnings. Warnings produce one warning notification rather than a second
message after success. Multi-step callers suppress intermediate notifications
and publish one final summary while preserving execution and batch records.

| Area | Feedback ownership |
| --- | --- |
| Launch | Transient save/delete/copy/request results; no persistent action rows |
| Environment | Immediate comparison summary; full report remains on the page |
| Skills and Sources | Resource execution or one multi-step/batch summary |
| MCP | Resource execution, one multi-target write summary, explicit connection-test result |
| Rules | Resource execution with follow/backup details in the same notification |
| Programs | Action dialog result and batch version result; preference failures prevent success |
| Settings | Save, login-entry and update results; field/read errors remain local |
| Activity | Transient clear result; full execution and failure records remain available |
| Assistants and MCP variables | Shared loading, read-error/retry and empty-state components |
| Tips and extension backups | Explicit copy/import/export feedback; original content remains unchanged |

[PageState](../../app/src/components/PageState.tsx) shows visible loading text and
read failures with retry. [EmptyState](../../app/src/components/EmptyState.tsx)
keeps guidance readable at constrained widths.
[ConfirmModal](../../app/src/components/ConfirmModal.tsx) keeps submission errors
outside the scrolling body, bounded above its action buttons, and announces busy
state. Dedicated editors keep validation near their inputs or fixed footer.

## Constraints and failure behavior

Action titles describe the attempted action. Diagnostics and recovery advice stay
in the description; recognized errors retain their original text. Page reads do
not announce success on mount. Partial work, unknown checks, cancelled inspection
and unverified correction cannot show a complete-success notification.

Notification expiry never removes a report or changes resources. Failed operations
do not retry automatically. Existing confirmation, preview, identity-isolation and
authorization behavior is unchanged. No global language/time-zone defaults are
introduced; tests use mock or isolated state and preserve personal system settings.

## Verification

```sh
pnpm --dir app test:unit run src/test/notifications.test.tsx src/test/ConfirmModal.test.tsx src/test/BatchApply.test.tsx src/test/ProgramPinFeedback.test.tsx src/test/queries.test.tsx src/test/UiCopyGuard.test.ts --maxWorkers=1
pnpm --dir app test:unit run --maxWorkers=1
pnpm --dir app build
```

Regressions check modal placement, defaults, original diagnostics, callback
preservation, one-summary batches, partial and unknown outcomes, MCP connection
results and multi-target writes, version-preference failure, form footer location
and the single notification entry point. Browser acceptance checks both languages
at 1000 × 650 and 760 × 650, including empty lists and long stacked diagnostics.
Native acceptance checks the real bridge's failed-launch notification with an
unsaved missing-path draft; no real application needs to start.
