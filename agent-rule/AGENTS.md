# AGENTS.md

## Scope
- Edit shared rules in `agent-rule/rules/src/`.
- Treat `agent-rule/rules/dist/` as generated output only.

## Workflow
- After changing `rules/src/`, run `make rules-build`.
- If the new output needs to be distributed, run `make rules:apply`.

## Boundaries
- Never hand-edit files in `agent-rule/rules/dist/`.
- Keep `rules/src/shared/core.md` limited to always-on cross-agent rules.
- Put low-frequency workflow detail in skills maintained by `agent-skills`; keep agent-specific differences in `rules/src/agents/*/adapter.md`.
- Preserve the shared core bias toward explicit assumptions, simplest-change, surgical-edit, and verification-driven execution.
- Keep this file narrow and do not repeat the repo-wide setup or test guidance.
