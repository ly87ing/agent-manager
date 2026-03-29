# AGENTS.md

## Scope
- Use `python3 -m unittest tests.test_project_contracts -v` for the contract-test slice.
- Use the most specific unittest target possible when iterating on one test.

## Fixture discipline
- Keep fixtures under `tests/fixtures/`.
- Prefer direct reads of repo files over mocks when a contract is about on-disk behavior.
- keep each test focused on one contract and one expectation.

## Clarity
- Name tests after the behavior they enforce.
- Make contract assertions explicit so failures point at the missing rule or file.
