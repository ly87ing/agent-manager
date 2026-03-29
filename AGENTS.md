# AGENTS.md

## Setup
- Python 3 and make are enough for the repo checks; no extra bootstrap is needed for doc or contract edits.
- If you touch rules, edit `agent-rule/rules/src/` and rebuild with `make rules-build`.

## Test commands
- `python3 -m unittest discover -s tests -p 'test*.py' -v`
- `python3 -m unittest tests.test_project_contracts -v`
- `make smoke`

## done-when
- The targeted test passes first, then the relevant broader test command passes.
- README and docs match the implemented behavior.
- No generated file was hand-edited.

## generated-file boundaries
- Treat `agent-rule/rules/dist/` as generated output only.
- Rebuild from `agent-rule/rules/src/` and sync from the build output instead of editing dist files directly.
