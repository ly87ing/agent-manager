# Frontend Guidelines

Focus on performance and accessibility. Ensure all dynamic data is validated.

## Playwright Execution Rules
- **Default to Headless**: All Playwright CLI invocations MUST use headless mode by default. This is essential for CI/CD environments, batch execution, and minimizing resource usage without disrupting the user's workspace.
- **Headed for Debugging Only**: Only explicitly append `--headed` (or use `headless: false` in configurations) when visually debugging complex interactions or UI state issues, or when specifically requested by the user. Ensure this is reverted before final completion unless it's a specific requirement (like testing browser extensions).

When frontend validation uses Playwright or browser automation, follow `artifact-hygiene.md` for output placement and cleanup instead of creating ad hoc repository artifacts.
