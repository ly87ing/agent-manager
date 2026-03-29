# Artifact Hygiene Guidelines

Prefer the target project's existing conventions over universal output rules. Choose output locations based on whether the files are maintained by the team or consumed by project workflows.

## Output Classification

- **Project assets:** files intentionally maintained by the team or expected to be reused, such as source code, shared helpers, stable fixtures, maintained config, approved snapshots, and durable documentation.
- **Workflow-consumed artifacts:** generated files that may be transient but are already expected by repository workflows such as config, scripts, or CI, or that the user explicitly asked to standardize as a reusable project convention.
- **Disposable run artifacts:** ad hoc debug scripts, copied reports, temporary screenshots, traces, videos, downloads, storage state, JSON/HTML dumps, and investigation notes that only serve the current run and are not referenced by project workflows.

## Required Workflow

1. Classify the expected output before running the tool.
2. If the project already defines a location or naming convention for that output, follow it.
3. If the output is a project asset, store it in the maintained project path and keep it cleanly reviewable.
4. If the output is generated and already consumed by repository workflows, prefer the project's standard gitignored output directory inside the working tree.
5. If the repository does not already consume that output, establish a new project-local convention only when the user explicitly asks for one.
6. If reuse or workflow consumption is uncertain, classify the output as a disposable run artifact.
7. If the output is disposable and no workflow depends on a working-tree path, use `mktemp -d`, `$TMPDIR`, or `/tmp`.
8. Do not create or widen `.gitignore`, config conventions, or repository directories just to host disposable outputs for the current task.
9. If a disposable artifact must temporarily land in a project-local ignored directory, delete it before completion unless the user explicitly asks to keep it.
10. Never place disposable artifacts in tracked paths or likely-to-be-committed locations such as the repository root, `docs/`, `scripts/`, `tests/fixtures/`, or ad hoc folders created for a single run.

## Playwright-Specific Rules

Before invoking `playwright-cli` or Playwright test commands, classify the expected output as a project asset, workflow-consumed artifact, or disposable run artifact.

- **Usually project assets:** test specs, page objects, shared fixtures, helper utilities, stable config changes, and intentionally maintained visual snapshots.
- **Usually workflow-consumed artifacts:** `playwright-report/`, `test-results/`, `blob-report/`, JUnit or JSON result files, and other outputs explicitly referenced by project scripts or CI.
- **Usually disposable run artifacts:** one-off traces, videos, screenshots, downloads, copied reports, temporary auth state, and ad hoc HAR, JSON, or storage dumps created only for a local investigation.

## Playwright Defaults

- Start from the repository's Playwright config, CI pipeline, and reporting conventions before adding output paths.
- Treat standard ignored directories such as `playwright-report/`, `test-results/`, and `blob-report/` as workflow-consumed locations only when repository workflows actually use them, not as storage for one-off investigation files.
- Do not update the target project's `.gitignore` or invent a repository output convention just to preserve disposable Playwright artifacts.
- If a disposable experiment is worth keeping, promote it into a maintained project asset and delete the disposable original.

## Decision Rule

When unsure, prefer established project conventions. Use project-local ignored output directories only for workflow-consumed artifacts already used by repository workflows, or when the user explicitly asks to create a reusable project convention. Otherwise, treat the output as disposable and use OS temp directories unless a working-tree path is required.
