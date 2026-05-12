# Artifact Hygiene Guidelines

Prefer the target project's existing conventions over universal output rules. Choose locations by whether files are maintained by the team or already consumed by repository workflows.

## Output Classification

- **Project assets:** team-maintained files — source code, shared helpers, stable fixtures, maintained config, approved snapshots, durable docs.
- **Workflow-consumed artifacts:** generated files that repository workflows (CI, scripts, config) already depend on, or that the user explicitly asks to standardize.
- **Disposable run artifacts:** ad hoc debug scripts, copied reports, temporary screenshots, traces, videos, downloads, storage state, JSON/HTML dumps, and investigation notes that only serve the current run.

## Where Each Goes

- Project asset → the maintained project path; keep it cleanly reviewable.
- Workflow-consumed → the project's standard gitignored output directory.
- Disposable with no required working-tree path → `mktemp -d`, `$TMPDIR`, or `/tmp`.
- Uncertain → treat as disposable.

## Constraints

- Don't create or widen `.gitignore`, config conventions, or repository directories just to host disposable outputs.
- Don't invent a new project-local convention unless the user explicitly asks for one.
- Never place disposable artifacts in tracked paths or likely-to-be-committed locations (repo root, `docs/`, `scripts/`, `tests/fixtures/`, ad hoc folders).
- If a disposable artifact must temporarily land in a project-local ignored directory, delete it before completion unless the user asks to keep it.

## Playwright Specifics

Before invoking `playwright-cli` or Playwright commands, classify the output:

- **Project assets:** test specs, page objects, shared fixtures, helper utilities, stable config changes, intentionally maintained visual snapshots.
- **Workflow-consumed:** `playwright-report/`, `test-results/`, `blob-report/`, JUnit/JSON result files actually used by project workflows.
- **Disposable:** one-off traces, videos, screenshots, downloads, copied reports, temporary auth state, ad hoc HAR/JSON/storage dumps for local investigation.

Defaults:

- Start from the repository's Playwright config, CI pipeline, and reporting conventions before adding output paths.
- Treat `playwright-report/`, `test-results/`, `blob-report/` as workflow-consumed only when workflows actually use them.
- Don't update `.gitignore` or invent output conventions for disposable artifacts.
- If a disposable experiment is worth keeping, promote it to a maintained asset and delete the original.

## Decision Rule

When unsure, prefer established project conventions. Use project-local ignored output directories only for workflow-consumed artifacts already used by repository workflows, or when the user explicitly asks to create a reusable project convention. Otherwise, treat the output as disposable and use OS temp directories unless a working-tree path is required.
