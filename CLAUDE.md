# CLAUDE.md

Repository collaboration follows the root @AGENTS.md. This is an import: its contents are loaded with this file, so no separate read is needed. The current goal is a native Rust backend and Tauri UI. The Python implementation is no longer maintained and has been approved for removal.

- Product scope and phases: [parallel account design](docs/MULTI-ACCOUNT-DESIGN.md). Data semantics and write protection: [GUI redesign](docs/GUI-REDESIGN.md).
- Current implementation, implementer changes, and acceptance: [task document](docs/goal-rust-gui.md).
- Rule text is maintained in `rules/` in the agent-skills repository (see its `rules/README.md`). Read the directory instructions when modifying retired `tests/`.
- The old `manage.sh` pipeline, three-layer JSON, automatic convergence, and Python packaging records are not requirements for the new version. Do not use them to restore retired behavior.
- Acceptance of old packages or workspace candidates does not prove the native version is complete. User configuration, sessions, and credentials are outside the code-cleanup scope.
