# Frontend Guidelines

Validate dynamic or external data at boundaries, and verify accessibility-relevant states when UI behavior changes.

## Browser Tool Selection Rules
- **Choose by Intent, Not Tool Availability**: Use Playwright for reproducible assertions, regression coverage, route/network mocking, and CI-stable automation. Use Chrome DevTools for one-off visual inspection, layout/scroll diagnosis, performance traces, Core Web Vitals, request waterfalls, cache/timing analysis, memory, and low-level WebSocket inspection. Use `agent-browser` for persistent multi-step web tasks needing navigation, auth, forms, scraping, or session reuse.
- **Preserve Authenticated Context Deliberately**: For authenticated or profile-dependent flows, use a session-aware browser tool (`agent-browser` in this repo) or attach to an existing Chrome session; never assume a fresh isolated DevTools profile has the right state.
- **Keep Playwright Headless by Default**: Run headless unless debugging visual state or the user explicitly asks for headed. Revert headed-only settings before completion unless that is the requested behavior.
- **Do Not Replace Regression Coverage**: DevTools and `agent-browser` are for diagnosis and task execution, not substitutes for Playwright assertions. If they surface a durable user-visible bug, add or update the smallest relevant Playwright check unless the user explicitly wants ad hoc investigation only.
- **Split Network Work by Intent**: Playwright for deterministic request assertions, waiting, interception, mocking; DevTools for waterfall, cache, priority, initiator, connection, timing.
- **Fail Explicitly Before Fallback**: If DevTools or `agent-browser` cannot start, connect, or attach, report the tool failure explicitly before falling back to Playwright or shell-built browser scripts.

When frontend validation uses Playwright or browser automation, follow `artifact-hygiene.md` for output placement and cleanup.
