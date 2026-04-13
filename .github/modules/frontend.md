# Frontend Guidelines

Focus on performance and accessibility. Ensure all dynamic data is validated.

## Browser Tool Selection Rules
- **Choose by Intent, Not Raw Capability**: Default to Playwright when the goal is reproducible frontend validation or regression coverage. Use `agent-browser` when the goal is for the agent to complete a persistent, multi-step website task. Use Chrome DevTools when the goal is browser-level diagnosis.

## Playwright Execution Rules
- **Default to Headless**: All Playwright CLI invocations MUST use headless mode by default. This is essential for CI/CD environments, batch execution, and minimizing resource usage without disrupting the user's workspace.
- **Headed for Debugging Only**: Only explicitly append `--headed` (or use `headless: false` in configurations) when visually debugging complex interactions or UI state issues, or when specifically requested by the user. Ensure this is reverted before final completion unless it's a specific requirement (like testing browser extensions).
- **Playwright First for Reproducible Frontend Debugging**: Default to Playwright for frontend functional validation, browser automation, reproducible debugging, trace collection, and cross-browser checks. Prefer Playwright CLI, `--debug`, `--ui`, and trace tooling before falling back to manual browser poking.
- **Keep Automated Checks in Playwright**: Use Playwright for assertions and automation around user-visible behavior, locator or actionability failures, route mocking, `waitForResponse`, API expectations, and other checks that should stay reproducible locally or in CI.

## agent-browser Usage Rules
- **Use agent-browser for Agent-Driven Web Tasks**: Use `agent-browser` when the user wants the agent to navigate websites, authenticate, fill forms, scrape data, or complete persistent multi-step web tasks directly, especially when session continuity and ref-based interaction matter more than producing a reusable regression test.
- **Prefer agent-browser Over Manual Browser Poking for Task Execution**: When the job is to have the agent operate a website end-to-end, prefer `agent-browser` over manual browser clicking or DevTools panel interaction.
- **Do Not Replace Playwright Regression Checks**: `agent-browser` is for task execution and investigation, not a substitute for Playwright assertions or stable regression coverage. If `agent-browser` reveals a durable user-visible bug or flow worth protecting, add or update the smallest relevant Playwright check unless the user explicitly wants ad hoc execution only.

## Chrome DevTools Escalation Rules
- **Use Chrome DevTools for Browser-Level Diagnosis**: Use Chrome DevTools, or the agent's Chrome DevTools integration when available, for performance traces, Core Web Vitals, long tasks, layout shift, paint or rendering analysis, memory issues, request waterfall inspection, headers or timing breakdowns, cache behavior, initiator chains, throttling, and low-level WebSocket inspection.
- **Prefer Root-Cause Analysis Over Ad Hoc Automation**: When the goal is to diagnose performance or network behavior rather than preserve a regression test, prefer Chrome DevTools over writing throwaway Playwright code.
- **Do Not Replace Regression Coverage**: Chrome DevTools is a diagnosis tool, not a substitute for Playwright regression coverage. If DevTools work reveals a stable user-facing bug or scenario worth protecting, add or update the smallest relevant Playwright check unless the user explicitly wants manual diagnosis only.
- **Split Network Work by Intent**: For network issues, use Playwright when you need deterministic request assertions, waiting, interception, or mocking. Use Chrome DevTools when you need waterfall, cache, priority, initiator, connection, or timing analysis.

When frontend validation uses Playwright or browser automation, follow `artifact-hygiene.md` for output placement and cleanup instead of creating ad hoc repository artifacts.
