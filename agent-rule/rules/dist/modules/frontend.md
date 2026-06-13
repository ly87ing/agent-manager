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

Set these as defaults up front. They are the lessons from delivering interactive docs; applying them on the first pass avoids rounds of "still too small" / "this is redundant" / "clicking does nothing".

- **Accessibility-first type.** Default base font ≥ 18px with generous line-height (~1.7) and padding; size for small screens and older / low-vision readers, not for a large desktop monitor. Never ship sub-14px body text. When unsure, go larger; add an A−/A+ control if the audience is broad.
- **Self-contained & offline by default.** For enterprise / air-gapped / 信创 / 内网 audiences, ship a single file with zero external dependencies — no CDN, web fonts, or remote diagram libraries. Hand-draw diagrams as inline SVG/CSS, or bundle the library inline.
- **Detail-on-demand = centered modal overlay.** When clicking an element reveals detail, pop a fixed centered overlay (dismiss via ×, backdrop click, and Esc; lock body scroll while open; don't auto-open on load). Do NOT dock the detail in a panel the user must scroll to — that defeats the click.
- **One canonical visual per flow.** Show a given flow/sequence once; don't duplicate it as both a card row and a diagram. Make the single visual the interactive entry point.
- **Make the whole target clickable.** Give clickable diagram elements a full hit area (e.g. a transparent full-width/row rect with `pointer-events:all`), not just a 1–2px line or a tiny glyph.
- **Verify the rendered result before "done".** Load it headless (Chrome DevTools / Playwright): assert no console errors, key elements present, and review a screenshot. Never declare an HTML deliverable done from unrendered markup alone.
