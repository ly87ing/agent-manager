# Agent Manager Usability Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the project safer and easier to use by aligning defaults, clarifying the effective config model, making terminal takeover opt-in, exposing provider choice in the UI, and removing the author-specific secrets repo assumption.

**Architecture:** Keep the existing policy/definition split, but move ambiguous or high-impact behavior behind explicit policy flags and explicit documentation. Implement the behavior changes through small runtime edits in `manage.sh`, `src/interactive.py`, and `agent-terminal/sync_terminal.py`, with matching contract tests and docs updates so the user-facing model stays coherent.

**Tech Stack:** Bash, Python 3 `unittest`, JSON config, project contract tests

---

### Task 1: Safe Default Entry Point

**Files:**
- Modify: `manage.sh`
- Modify: `tests/test_manage.py`

- [ ] **Step 1: Write the failing test**

Add a test that runs `./manage.sh --dry-run --offline` in a copied repo and asserts it behaves like help text instead of silently defaulting to `all`.

- [ ] **Step 2: Run test to verify it fails**

Run: `python3 -m unittest tests.test_manage.ManageScriptTests.test_manage_without_command_shows_help_and_does_not_run_pipeline -v`
Expected: FAIL because current runtime defaults to `all`.

- [ ] **Step 3: Write minimal implementation**

Change `manage.sh` so missing command defaults to `help`, preserving explicit `all` / `bootstrap` behavior.

- [ ] **Step 4: Run test to verify it passes**

Run: `python3 -m unittest tests.test_manage.ManageScriptTests.test_manage_without_command_shows_help_and_does_not_run_pipeline -v`
Expected: PASS

### Task 2: Terminal Sync Opt-In

**Files:**
- Modify: `config/manager.json`
- Modify: `manage.sh`
- Modify: `agent-terminal/sync_terminal.py`
- Modify: `tests/test_manage.py`
- Modify: `tests/test_sync_terminal.py`

- [ ] **Step 1: Write the failing tests**

Add one `manage.sh` test proving `sync` skips terminal sync when terminal policy is absent/disabled, and one terminal-engine test proving disabled targets are skipped.

- [ ] **Step 2: Run tests to verify they fail**

Run:
- `python3 -m unittest tests.test_manage.ManageScriptTests.test_sync_skips_terminal_when_terminal_policy_disabled -v`
- `python3 -m unittest tests.test_sync_terminal.SyncTerminalTests.test_disabled_target_is_skipped_without_writing -v`

Expected: FAIL because terminal sync is currently unconditional and target configs have no enable gate.

- [ ] **Step 3: Write minimal implementation**

Add policy-aware gating for terminal sync, plus per-target `enabled` support in `terminal.json` consumption. Default the shipped terminal targets to disabled so terminal takeover becomes explicit opt-in.

- [ ] **Step 4: Run tests to verify they pass**

Run the two tests above.
Expected: PASS

### Task 3: Interactive Config Completeness

**Files:**
- Modify: `src/interactive.py`
- Modify: `tests/test_interactive.py`

- [ ] **Step 1: Write the failing tests**

Add tests covering agent `preferred_provider` editing and menu copy that explains edits are saved to `manager.local.json`.

- [ ] **Step 2: Run tests to verify they fail**

Run:
- `python3 -m unittest tests.test_interactive.InteractiveWorkflowTests.test_agent_edit_persists_preferred_provider -v`
- `python3 -m unittest tests.test_interactive.InteractiveWorkflowTests.test_main_menu_mentions_local_override_file -v`

Expected: FAIL because UI does not currently expose provider editing or explain the local override model.

- [ ] **Step 3: Write minimal implementation**

Extend the agent editor to list allowed providers from `config/agents.json`, persist the selected provider to `manager.local.json`, and update UI copy to explain the local override file.

- [ ] **Step 4: Run tests to verify they pass**

Run the two tests above.
Expected: PASS

### Task 4: Secrets Repository Configurability

**Files:**
- Modify: `scripts/init-secrets.sh`
- Modify: `tests/test_project_contracts.py`
- Modify: `tests/test_manage.py`
- Modify: `docs/SECRETS-MANAGEMENT.md`
- Modify: `docs/TROUBLESHOOTING.md`
- Modify: `docs/MCP-CONFIG-GUIDE.md`

- [ ] **Step 1: Write the failing tests**

Add contract tests proving the project no longer hardcodes the author repo as the documented default and that `init-secrets.sh` requires an explicit `SECRETS_REPO_URL` or positional URL when `secrets/` is missing.

- [ ] **Step 2: Run tests to verify they fail**

Run:
- `python3 -m unittest tests.test_project_contracts.ProjectContractTests.test_init_secrets_requires_explicit_repo_configuration -v`
- `python3 -m unittest tests.test_manage.ManageScriptTests.test_update_reports_missing_secrets_repo_configuration -v`

Expected: FAIL because the script still falls back to `https://github.com/ly87ing/secrets.git`.

- [ ] **Step 3: Write minimal implementation**

Remove the hardcoded default repo, make the script fail closed with a clear message when no repo URL is configured, and update docs to describe explicit configuration via `SECRETS_REPO_URL` or an argument.

- [ ] **Step 4: Run tests to verify they pass**

Run the two tests above.
Expected: PASS

### Task 5: Docs and Contract Alignment

**Files:**
- Modify: `README.md`
- Modify: `docs/ARCHITECTURE.md`
- Modify: `docs/TROUBLESHOOTING.md`
- Modify: `docs/SECRETS-MANAGEMENT.md`
- Modify: `docs/MCP-CONFIG-GUIDE.md`
- Modify: `tests/test_project_contracts.py`

- [ ] **Step 1: Write/update failing contract tests**

Add or extend project contract tests so README / architecture mention `manager.local.json`, explain terminal config as opt-in, and keep command guidance aligned with the new safe defaults.

- [ ] **Step 2: Run tests to verify they fail**

Run: `python3 -m unittest tests.test_project_contracts -v`
Expected: FAIL until docs are updated.

- [ ] **Step 3: Write minimal documentation changes**

Update user-facing docs so the effective configuration model, terminal opt-in, and secrets repo configuration are clearly documented without contradicting runtime behavior.

- [ ] **Step 4: Run tests to verify they pass**

Run: `python3 -m unittest tests.test_project_contracts -v`
Expected: PASS

### Task 6: Final Verification

**Files:**
- Modify as needed from previous tasks only

- [ ] **Step 1: Run focused behavior tests**

Run:
- `python3 -m unittest tests.test_manage -v`
- `python3 -m unittest tests.test_interactive -v`
- `python3 -m unittest tests.test_sync_terminal -v`
- `python3 -m unittest tests.test_project_contracts -v`

Expected: PASS

- [ ] **Step 2: Run smoke validation**

Run: `make smoke`
Expected: PASS

- [ ] **Step 3: Summarize residual risk**

Document any remaining gaps, especially around full-suite runtime behavior and untested interactive paths.
