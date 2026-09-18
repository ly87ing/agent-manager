# AGENTS.md

## Scope
- Edit shared rules in `agent-rule/rules/src/`.

## Workflow
- `rules:apply` renders shared core + adapter straight to each agent's rules file; there is no build or dist step. Editing `rules/src/` changes the defaults compiled into the engine, not what a machine already has in SQLite.

## Boundaries
- Keep `rules/src/shared/core.md` limited to always-on cross-agent rules.
- Put low-frequency workflow detail in skills maintained by `agent-skills`; keep agent-specific differences in `rules/src/agents/*/adapter.md`.
- Preserve the shared core bias toward explicit assumptions, simplest-change, surgical-edit, and verification-driven execution.
- Keep this file narrow and do not repeat the repo-wide setup or test guidance.
