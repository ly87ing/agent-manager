# CLAUDE.md

## Scope

- Prefer the narrowest unittest target, e.g. `python3 -m unittest tests.test_project_contracts -v`.
- When changing a single contract, narrow further to the specific test method.

## Fixture discipline

- Keep fixtures under `tests/fixtures/`.
- Prefer reading repo files directly for file contracts; use mocks sparingly.
- keep each test focused on one contract and one expectation.

## Clarity

- Test names describe the protected behavior directly.
- Write assertions explicitly enough that a failure points straight at the missing rule or doc constraint.
