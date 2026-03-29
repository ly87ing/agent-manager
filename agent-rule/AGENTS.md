# AGENTS.md

## Scope
- Edit shared rules in `agent-rule/rules/src/`.
- Treat `agent-rule/rules/dist/` as generated output only.

## Workflow
- After changing `rules/src/`, run `make rules-build`.
- If the new output needs to be distributed, run `make rules:sync`.

## Boundaries
- Never hand-edit files in `agent-rule/rules/dist/`.
- Keep this file narrow and do not repeat the repo-wide setup or test guidance.
