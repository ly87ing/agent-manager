# CLAUDE.md

## Scope

- Maintain the shared core and agent adapters under `agent-rule/rules/src/`.
- Treat `agent-rule/rules/dist/` as generated artifacts; do not hand-edit them directly.

## Workflow

- After changing rules, run `make rules-build` first.
- Run `make rules:sync` when distribution is needed.
- When adjusting target paths or filenames, also check `config/rules-targets.list`.

## Guardrails

- Keep shared rules portable; leave agent-specific detail in each adapter.
- For repo-level commands or general background, return to the root `CLAUDE.md`.
