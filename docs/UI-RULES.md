# UI Rules

Rules every page, dialog, and panel of the desktop app follows. Check them on every page a change touches, not only the one in a report. Where a rule can be checked mechanically, `app/src/test/UiCopyGuard.test.ts` fails on a violation.

## Layout

Check at about 1000 px and 760 px wide in the mock dev server (`pnpm --dir app dev`, then open `/?mock=1`).

Native first-launch acceptance uses the committed window geometry (1000 × 650),
with isolated HOME/data and a separate application identifier. Do not override
the width or height in a preview configuration. Check both empty and populated
pages within that viewport; a tall full-page capture does not prove default-window usability.

1. No horizontal scroll bar anywhere. The top navigation folds extra pages into More; content never overflows.
2. Key information (launch commands, repository URLs, versions) is shown in full and wraps at spaces. Only secondary information may truncate, and then it carries a hover title.
   Action buttons and menu items display their complete labels. Use an explicit verb and object; prefer wrapping or moving controls to another line over truncation. Do not replace action text with ellipses or informal phrases such as “不用了”.
3. No column whose value is almost always the same. Show only the abnormal case, inline.
4. Nothing looks clickable unless it is. A note that is only a tooltip goes as small text under the table instead.
5. One item per row, fixed columns, headers aligned with rows, status tags on one line, and one primary action per row with the rest under ⋯ (see [Concurrent Accounts](MULTI-ACCOUNT-DESIGN.md) §3.6).
6. Buttons in a list sit in fixed-width columns so they line up from row to row.
7. While a background job runs, conflicting actions are disabled and say why.
8. Dialog confirm and cancel buttons are reachable without scrolling.

Empty states use short, left-aligned guidance with a small icon and an adjacent
action. The `compact` variant omits its border inside an existing panel. Secondary
read-only disclosures should not look like a second primary card.

## Controls and marks

9. Dropdowns use `components/Select`, never a native `<select>`: it looks the same in light and dark themes and on every platform's webview, and it renders its list above dialogs and tables without being clipped. Tests drive it with `chooseOption` and `readOptions` from `src/test/render-with-engine.tsx` and read the current value from `data-value`. *(Checked by the guard test.)*
10. Mark state with a dot or a coloured chip, not a coloured left-border stripe. *(Checked by the guard test.)*
11. A click never scrolls the page. Scrolling an element into view is only for arriving from another page with a target (for example Sources or Programs opened at a given item).
12. Information that sends data somewhere or runs something says where and what before the user confirms: the host a key or prompt goes to, the file a key is written to in plaintext, the command that will run.

## Copy

13. Interface text defaults to English and supports Simplified Chinese through Settings → Language. Translate application-owned labels, instructions and messages; preserve user content, resource names, paths and original engine diagnostics. Both languages avoid engineering jargon; the banned words are listed in the guard test.

Rules 9–12 were adopted on 2026-10-09 from a comparison with Magpie's GUI standards; rules 1–8 collect layout problems reported repeatedly during September 2026.

## Feedback

- Route action notifications through `lib/notifications.ts`. Success and information
  last four seconds; errors, warnings and undo actions last ten seconds. Detailed
  check summaries may explicitly keep ten seconds. Every notification can close.
- Keep the action title short. Put the cause, next step, effect conditions and
  original diagnostic in the description. Long text wraps and scrolls within its
  own bounded area. At most two notifications expand in the viewport.
- Page notifications appear at the bottom right. Dialog notifications appear at
  the top right so the confirm/cancel footer stays available.
- One user operation has one completion summary. Multi-step callers suppress
  intermediate success messages while keeping execution records and failure
  details. Partial completion and unknown evidence use warnings; they never
  announce complete success.
- Loading messages are visible. Initial read failures stay beside Retry; they do
  not create unsolicited notifications. Empty states explain the next useful action.
- Form errors stay beside the relevant field or in the fixed action footer.
  Submission failures also announce their outcome immediately when needed.
  Launch/copy errors expire without leaving persistent error rows. Check reports
  and operation records retain their complete results.
