# Agent Behavior Guidelines

## 1. Core Mandates

- **MANDATORY:** The agent **MUST** use Chinese (Simplified) for **ALL** interactions, outputs, and generated content.
  - This includes conversational responses, explanations, summaries, and analysis results.
  - **EXCEPTION:** Professional technical terms may remain in English when translation would reduce clarity.

## 2. Tool Strategy & Workflow

- **Primary Semantic Search:** Use the `codebase-retrieval` tool from Augment Context Engine (or similar high-level codebase search tools) as the default entry point when file locations are unclear or high-level context is needed.
- **Plan Before Editing:** Start with codebase retrieval to explore, analyze context and impact, and plan your approach *before* making non-trivial edits.
- **Exact Match Search:** Use local file tools or `rg` only for exact string matches, config keys, logs, or generated files.
- **Verification is CRITICAL:** Always run tests, lint, or other local verification after modification.
- **Artifact Hygiene:** Before invoking Playwright, browser automation, generators, or debug tooling, classify outputs as project assets, workflow-consumed artifacts, or disposable run artifacts. Follow the target project's established conventions first. Use project-local ignored output directories only for workflow-consumed artifacts already used by repository workflows, or when the user explicitly asks to establish a reusable project convention. Otherwise, treat the output as disposable and use OS temp directories unless a working-tree path is required.

## 3. Domain Rules (Progressive Disclosure)

**CRITICAL INSTRUCTION**: Before executing tasks in specific domains, you **MUST** read the corresponding domain rules using your file reading tools. The rules below are essential for ensuring high-quality, standardized outputs in their respective areas.

- **Architecture & System Design:**
  If your task involves creating new modules, modifying shared/common code, or architectural decisions, you **MUST** read:
  `modules/architecture.md` (Relative to this file's path, or typically located at `~/.<agent>/modules/architecture.md` or equivalent config directory).

- **Code Style & Integrity:**
  If your task involves writing new code, handling data/config, or adding comments, you **MUST** read:
  `modules/style.md`

- **Frontend Development & Verification:**
  If your task involves UI, frontend validation logic, user flows, or Playwright/browser automation execution, you **MUST** read:
  `modules/frontend.md`

- **Generated Output & Artifact Hygiene:**
  If your task involves `playwright-cli`, Playwright, browser automation, generators, generated test artifacts, or temporary debug artifacts, you **MUST** read:
  `modules/artifact-hygiene.md`
