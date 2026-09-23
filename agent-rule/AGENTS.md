# AGENTS.md

## Scope
- Edit shared rules in `agent-rule/rules/src/`.

## Workflow
- `rules/src/shared/core.md` is compiled into the engine as the read-only recommended rules; it reaches an agent's rules file only when the user applies it. The effective rules are each agent's real rules file; the engine no longer renders shared core + adapter, and the adapters are read only to migrate legacy saved sources into templates.

## Boundaries
- Keep `rules/src/shared/core.md` limited to always-on cross-agent rules.
- Put low-frequency workflow detail in skills maintained by `agent-skills`; keep agent-specific differences in `rules/src/agents/*/adapter.md`.
- Preserve the shared core bias toward explicit assumptions, simplest-change, surgical-edit, and verification-driven execution.
- Keep this file narrow and do not repeat the repo-wide setup or test guidance.
