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

## Interactive HTML / doc deliverables

Defaults for HTML handed to readers (decks, dashboards, interactive docs). Decide them before building, not after feedback.

- **Make text scale with the reader.** Use relative units (rem/em) and respect browser zoom and OS text-size; don't lock layout to px that can't scale. Pick a comfortable default for the audience — informational/doc decks lean larger, dense tools can be smaller — and don't ship cramped or sub-14px body text. Meet WCAG contrast and support 200% zoom without loss of content.
- **Match dependency strategy to the deployment.** Bundler / CDN / design-system is right for normal web apps; for air-gapped / enterprise / 信创 / 内网 delivery, ship a single self-contained file with zero external dependencies (no CDN, web fonts, or remote diagram libs) — inline SVG/CSS or bundle the library.
- **Surface click-triggered detail where the user is looking.** Don't make them scroll to find it. A centered modal, inline expansion, or a pane next to the trigger all work — choose by context. If you use a modal, trap focus, close on Esc and backdrop, restore focus on close, and label it (`role="dialog"` + `aria-modal`/`aria-label`).
- **One canonical visual per flow.** Show a flow/sequence once; don't duplicate it as both a card row and a diagram. Make the single visual the interactive entry point. (Mirrors `writing.md` "One fact, one place".)
- **Give interactive elements an adequate hit area.** Meet target-size guidance (WCAG 2.5.8); don't rely on a 1–2px line or a tiny glyph. For SVG, place a transparent full-row/area rect with `pointer-events:all` behind the visuals.
- **Verify the rendered result before "done".** Load it headless (Chrome DevTools / Playwright): assert no console errors, key elements present, interactions actually work, and review a screenshot. Never declare an HTML deliverable done from unrendered markup alone.
