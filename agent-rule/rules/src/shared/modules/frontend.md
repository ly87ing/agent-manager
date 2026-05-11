# Frontend Guidelines

Validate dynamic or external data at boundaries, and verify accessibility-relevant states when UI behavior changes.

## Browser Tool Selection Rules
- **Choose by Intent, Not Tool Availability**: Use Playwright for reproducible assertions, regression coverage, route or network mocking, CI-stable automation, and checks that should survive local reruns. Use Chrome DevTools for one-off visual inspection, layout or scroll diagnosis, performance traces, Core Web Vitals, request waterfalls, cache or timing analysis, memory, and low-level WebSocket inspection. Use `agent-browser` for persistent multi-step web tasks where the agent must navigate, authenticate, fill forms, scrape data, or reuse session continuity.
- **Preserve Authenticated Context Deliberately**: For authenticated or profile-dependent flows, prefer the session-aware browser tool (`agent-browser` in this repo) or explicitly attach to the existing Chrome session; do not silently assume a fresh isolated DevTools profile has the right state.
- **Keep Playwright Headless by Default**: Run Playwright CLI in headless mode unless debugging visual state or the user explicitly asks for headed mode. Revert headed-only settings before completion unless they are the requested behavior.
- **Do Not Replace Regression Coverage**: Chrome DevTools and `agent-browser` are for diagnosis and task execution, not substitutes for Playwright assertions or stable regression coverage. If they expose a durable user-visible bug or flow worth protecting, add or update the smallest relevant Playwright check unless the user explicitly wants ad hoc investigation only.
- **Split Network Work by Intent**: For network issues, use Playwright when you need deterministic request assertions, waiting, interception, or mocking. Use Chrome DevTools when you need waterfall, cache, priority, initiator, connection, or timing analysis.
- **Fail Explicitly Before Fallback**: If Chrome DevTools or `agent-browser` cannot start, connect, or attach, report the tool failure explicitly before falling back to Playwright or shell-built browser scripts.

When frontend validation uses Playwright or browser automation, follow `artifact-hygiene.md` for output placement and cleanup instead of creating ad hoc repository artifacts.
