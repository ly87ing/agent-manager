"""Unit and integration tests for Phase A scope boundaries (plan & apply).

Verifies:
1. Same-domain A & B both drifted in JSON: applying only A resolves A and preserves B.
2. Same-domain A & B both drifted in TOML: applying only A resolves A and preserves B.
3. Definition removal: removing overlay definition preserves unselected servers,
   plan displays leftover removal, and apply cleans up only the removed server.
4. Skills scoping: applying single skill with cached repo does not git-pull other skills,
   and unselected skill links are untouched.
5. Skill collection scoping: --only skill:<collection> covers both collection and child skills.
6. Tools scoping: applying single tool does not prune unselected tools.
7. Agents gate: unselected agent requiring uninstall does not block applying another agent.
8. Unknown and cross-domain scopes fail before writing with exit code 2.
9. Dry-run with --only makes zero writes.
"""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
COPY_IGNORE = shutil.ignore_patterns(
    "repositories", ".git", "__pycache__", "secrets", ".env", "app", "node_modules", "target", "manager.local.json", "manager.synced.json"
)


class ScopeBoundariesTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_scope_test_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)
        self.repo = self.temp_dir / "repo"
        self.home = self.temp_dir / "home"
        self.home.mkdir()
        shutil.copytree(ROOT, self.repo, ignore=COPY_IGNORE)
        engine_bin = ROOT / "engine-rs" / "target" / "release" / "agent-manager-engine"
        if engine_bin.exists():
            dest = self.repo / "engine-rs" / "target" / "release"
            dest.mkdir(parents=True, exist_ok=True)
            shutil.copy2(engine_bin, dest / "agent-manager-engine")
        for name in ("manager.local.json", "manager.synced.json"):
            (self.repo / "config" / name).unlink(missing_ok=True)
        self.bindir = self.temp_dir / "bin"
        self.bindir.mkdir()
        self.snapshot_dir = self.temp_dir / "snapshots"
        self.log_prefix = self.temp_dir / "logs" / "agent_manager_sync"
        self._write_minimal_configs()

    def _write_minimal_configs(self):
        self._write_config("manager.json", {"agents": {}, "tools": {}, "skills": {}, "mcp": {}, "browser_us_sync": {"enabled": False}})
        self._write_config("agents.json", {})
        self._write_config("tools.json", {"tools": {}})
        self._write_config("mcp.json", {"servers": {}, "agent_paths": {}})
        self._write_config("skills.json", {"repositories": {}, "agents": {}})
        (self.repo / "config" / "rules-targets.list").write_text("", encoding="utf-8")

    def _write_config(self, name, obj):
        (self.repo / "config" / name).write_text(json.dumps(obj, indent=2) + "\n", encoding="utf-8")

    def _env(self, extra=None):
        env = os.environ.copy()
        env["HOME"] = str(self.home)
        env["AGENT_MANAGER_LOCK_FILE"] = str(self.temp_dir / "agent_manager.lock")
        env["AGENT_MANAGER_LOG_PREFIX"] = str(self.log_prefix)
        env["AGENT_MANAGER_SNAPSHOT_DIR"] = str(self.snapshot_dir)
        env["AGENT_MANAGER_UPSTREAM_CACHE"] = str(self.temp_dir / "upstream-cache.json")
        env["AGENT_MANAGER_SYNCED_CONFIG"] = str(self.temp_dir / "manager.synced.json")
        env["AGENT_MANAGER_LOCAL_CONFIG"] = str(self.temp_dir / "manager.local.json")
        if extra:
            env.update(extra)
        return env

    def _run(self, *argv, env_extra=None):
        (self.temp_dir / "logs").mkdir(exist_ok=True)
        return subprocess.run(
            [str(self.repo / "manage.sh"), *argv],
            cwd=self.repo,
            env=self._env(env_extra),
            capture_output=True,
            text=True,
            check=False,
            timeout=120,
        )

    def _payload(self, *argv, expect_exit=0, env_extra=None):
        result = self._run(*argv, env_extra=env_extra)
        self.assertEqual(result.returncode, expect_exit, f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}")
        try:
            return json.loads(result.stdout)
        except json.JSONDecodeError:
            self.fail(f"Invalid JSON from command {argv}:\n{result.stdout}\nstderr:\n{result.stderr}")

    # =========================================================================
    # 1. MCP JSON: Same-domain A & B both drifted -> apply only A -> B preserved
    # =========================================================================

    def test_mcp_scope_preserves_unselected_drifted_in_json(self):
        target = self.home / ".claude.json"
        self._write_config(
            "mcp.json",
            {
                "servers": {
                    "alpha": {"command": "alpha-cmd", "args": ["--new-a"], "env": {}, "agents": ["claude-code"]},
                    "beta": {"command": "beta-cmd", "args": ["--new-b"], "env": {}, "agents": ["claude-code"]},
                },
                "agent_paths": {"claude-code": str(target)},
            },
        )
        self._write_config(
            "manager.json",
            {
                "agents": {}, "tools": {}, "skills": {},
                "mcp": {"alpha": {"enabled": True}, "beta": {"enabled": True}},
                "browser_us_sync": {"enabled": False},
            },
        )
        # Pre-seed target with BOTH servers drifted from the definition
        target.write_text(
            json.dumps({
                "mcpServers": {
                    "alpha": {"command": "alpha-cmd", "args": ["--old-a"], "env": {"_MANAGED_BY": "agent-manager"}},
                    "beta": {"command": "beta-cmd", "args": ["--old-b"], "env": {"_MANAGED_BY": "agent-manager"}},
                }
            }, indent=2),
            encoding="utf-8",
        )

        # Plan with --only mcp:alpha should show action for alpha, NOT beta
        plan_res = self._payload("mcp:plan", "--only", "mcp:alpha", "--json")
        actions = plan_res["data"]["actions"]
        self.assertTrue(any(a["element"] == "mcp:alpha" for a in actions))
        self.assertFalse(any(a["element"] == "mcp:beta" for a in actions))

        # Apply only alpha
        apply_res = self._payload("mcp:apply", "--only", "mcp:alpha", "--json", "--offline")
        self.assertTrue(apply_res["ok"])

        # Check target content: alpha was updated, beta remains exactly as it was
        content = json.loads(target.read_text(encoding="utf-8"))
        self.assertEqual(content["mcpServers"]["alpha"]["args"], ["--new-a"])
        self.assertEqual(content["mcpServers"]["beta"]["args"], ["--old-b"], "Unselected beta must remain drifted")
        self.assertEqual(content["mcpServers"]["beta"]["env"]["_MANAGED_BY"], "agent-manager")

    # =========================================================================
    # 2. MCP TOML: Same-domain A & B both drifted -> apply only A -> B preserved
    # =========================================================================

    def test_mcp_scope_preserves_unselected_drifted_in_toml(self):
        target = self.home / ".codex" / "config.toml"
        target.parent.mkdir(parents=True, exist_ok=True)
        self._write_config(
            "mcp.json",
            {
                "servers": {
                    "alpha": {"command": "alpha-cmd", "args": ["--new-a"], "env": {}, "agents": ["codex"]},
                    "beta": {"command": "beta-cmd", "args": ["--new-b"], "env": {}, "agents": ["codex"]},
                },
                "agent_paths": {"codex": {"path": str(target), "format": "toml"}},
            },
        )
        self._write_config(
            "manager.json",
            {
                "agents": {}, "tools": {}, "skills": {},
                "mcp": {"alpha": {"enabled": True}, "beta": {"enabled": True}},
                "browser_us_sync": {"enabled": False},
            },
        )
        # Pre-seed target with BOTH servers drifted
        target.write_text(
            "[mcp_servers.\"alpha\"]\ncommand = \"alpha-cmd\"\nargs = [\"--old-a\"]\n[mcp_servers.\"alpha\".env]\n_MANAGED_BY = \"agent-manager\"\n\n"
            "[mcp_servers.\"beta\"]\ncommand = \"beta-cmd\"\nargs = [\"--old-b\"]\n[mcp_servers.\"beta\".env]\n_MANAGED_BY = \"agent-manager\"\n",
            encoding="utf-8",
        )

        apply_res = self._payload("mcp:apply", "--only", "mcp:alpha", "--json", "--offline")
        self.assertTrue(apply_res["ok"])

        # Check target content
        content = target.read_text(encoding="utf-8")
        self.assertIn("args = [\"--new-a\"]", content)
        self.assertIn("args = [\"--old-b\"]", content, "Unselected beta in TOML must remain drifted and not pruned")
        self.assertIn("[mcp_servers.\"beta\"]", content)

    # =========================================================================
    # 3. Definition Removal: Deleted/tombstoned server cleanup with scope
    # =========================================================================

    def test_mcp_scope_cleans_up_removed_definition_via_evidence(self):
        target = self.home / ".claude.json"
        self._write_config(
            "mcp.json",
            {
                "servers": {
                    "beta": {"command": "beta-cmd", "args": ["--new-b"], "env": {}, "agents": ["claude-code"]},
                },
                "agent_paths": {"claude-code": str(target)},
            },
        )
        self._write_config(
            "manager.json",
            {
                "agents": {}, "tools": {}, "skills": {},
                "mcp": {"beta": {"enabled": True}},
                "browser_us_sync": {"enabled": False},
            },
        )
        # alpha was previously managed and is still in .claude.json, but has been removed from definitions
        target.write_text(
            json.dumps({
                "mcpServers": {
                    "alpha": {"command": "alpha-cmd", "args": ["--old-a"], "env": {"_MANAGED_BY": "agent-manager"}},
                    "beta": {"command": "beta-cmd", "args": ["--old-b"], "env": {"_MANAGED_BY": "agent-manager"}},
                }
            }, indent=2),
            encoding="utf-8",
        )

        # Plan for removed server alpha should recognize it from observed managed evidence and show removal
        plan_res = self._payload("mcp:plan", "--only", "mcp:alpha", "--json")
        actions = plan_res["data"]["actions"]
        self.assertEqual(len(actions), 1)
        self.assertEqual(actions[0]["element"], "mcp:alpha")
        self.assertIn("移除", actions[0]["note"])

        # Apply only alpha removal
        apply_res = self._payload("mcp:apply", "--only", "mcp:alpha", "--json", "--offline")
        self.assertTrue(apply_res["ok"])

        content = json.loads(target.read_text(encoding="utf-8"))
        self.assertNotIn("alpha", content["mcpServers"])
        self.assertIn("beta", content["mcpServers"])
        self.assertEqual(content["mcpServers"]["beta"]["args"], ["--old-b"], "Unselected beta was not modified")

    # =========================================================================
    # 4. Skills Scoping: Single skill with cached repo does not git-pull
    # =========================================================================

    def test_skills_scope_single_skill_does_not_pull_cached_repo(self):
        repo_base = self.repo / "agent-skill" / "repositories" / "my-repo"
        repo_base.mkdir(parents=True, exist_ok=True)
        (repo_base / ".git").mkdir(exist_ok=True)
        # Create two skills in repo
        (repo_base / "skill-a").mkdir()
        (repo_base / "skill-a" / "SKILL.md").write_text("---\nname: skill-a\n---\n", encoding="utf-8")
        (repo_base / "skill-b").mkdir()
        (repo_base / "skill-b" / "SKILL.md").write_text("---\nname: skill-b\n---\n", encoding="utf-8")

        agent_skills = self.home / ".claude" / "skills"
        agent_skills.mkdir(parents=True, exist_ok=True)

        self._write_config(
            "skills.json",
            {
                "repositories": {
                    "my-repo": {
                        "url": "https://example.invalid/my-repo.git",
                        "agents": ["claude-code"],
                        "auto_update": True,
                    }
                },
                "agents": {"claude-code": str(agent_skills)},
            },
        )
        self._write_config(
            "manager.json",
            {
                "agents": {}, "tools": {}, "mcp": {},
                "skills": {"my-repo": {"enabled": True}},
                "browser_us_sync": {"enabled": False},
            },
        )

        # Plan for single skill: should NOT contain git-pull when cached
        plan_res = self._payload("skills:plan", "--only", "skill:my-repo/skill-a", "--json")
        actions = plan_res["data"]["actions"]
        self.assertFalse(any(a["op"] == "git-pull" for a in actions), "Must not git pull cached repo for single skill")
        self.assertTrue(any(a["element"] == "skill:my-repo/skill-a" for a in actions))
        self.assertFalse(any(a["element"] == "skill:my-repo/skill-b" for a in actions))

        # Apply only skill-a
        apply_res = self._payload("skills:apply", "--only", "skill:my-repo/skill-a", "--json", "--offline")
        self.assertTrue(apply_res["ok"])
        self.assertTrue((agent_skills / "skill-a").exists())
        self.assertFalse((agent_skills / "skill-b").exists(), "skill-b link must not be created")

    # =========================================================================
    # 5. Skills Scoping: --only skill:<collection> covers both repo and children
    # =========================================================================

    def test_skills_scope_collection_covers_collection_and_children(self):
        repo_base = self.repo / "agent-skill" / "repositories" / "my-repo"
        repo_base.mkdir(parents=True, exist_ok=True)
        (repo_base / "skill-a").mkdir()
        (repo_base / "skill-a" / "SKILL.md").write_text("---\nname: skill-a\n---\n", encoding="utf-8")
        (repo_base / "skill-b").mkdir()
        (repo_base / "skill-b" / "SKILL.md").write_text("---\nname: skill-b\n---\n", encoding="utf-8")

        agent_skills = self.home / ".claude" / "skills"
        agent_skills.mkdir(parents=True, exist_ok=True)

        self._write_config(
            "skills.json",
            {
                "repositories": {
                    "my-repo": {
                        "url": "https://example.invalid/my-repo.git",
                        "agents": ["claude-code"],
                    }
                },
                "agents": {"claude-code": str(agent_skills)},
            },
        )
        self._write_config(
            "manager.json",
            {
                "agents": {}, "tools": {}, "mcp": {},
                "skills": {"my-repo": {"enabled": True}},
                "browser_us_sync": {"enabled": False},
            },
        )

        plan_res = self._payload("skills:plan", "--only", "skill:my-repo", "--json")
        actions = plan_res["data"]["actions"]
        elements = {a["element"] for a in actions}
        self.assertIn("skill:my-repo/skill-a", elements)
        self.assertIn("skill:my-repo/skill-b", elements)

    # =========================================================================
    # 6. Tools Scoping: Applying single tool does not prune unselected tools
    # =========================================================================

    def test_tools_scope_preserves_unselected_tool_directories(self):
        tools_base = self.repo / "agent-tool"
        (tools_base / "tool-a").mkdir(parents=True, exist_ok=True)
        (tools_base / "tool-a" / ".manager-owned").write_text("tool-a\n", encoding="utf-8")
        (tools_base / "tool-b").mkdir(parents=True, exist_ok=True)
        (tools_base / "tool-b" / ".manager-owned").write_text("tool-b\n", encoding="utf-8")

        self._write_config(
            "tools.json",
            {
                "tools": {
                    "tool-a": {"ready_check": "bin:true", "install": "echo install-a", "upgrade": "echo upgrade-a"},
                    "tool-b": {"ready_check": "bin:true", "install": "echo install-b", "upgrade": "echo upgrade-b"},
                }
            },
        )
        self._write_config(
            "manager.json",
            {
                "agents": {}, "skills": {}, "mcp": {},
                "tools": {"tool-a": {"enabled": True}, "tool-b": {"enabled": True}},
                "browser_us_sync": {"enabled": False},
            },
        )

        apply_res = self._payload("tools:apply", "--only", "tools:tool-a", "--json", "--offline")
        self.assertTrue(apply_res["ok"])
        self.assertTrue((tools_base / "tool-a").exists())
        self.assertTrue((tools_base / "tool-b").exists(), "tool-b must not be pruned when only applying tool-a")

    # =========================================================================
    # 7. Agents Gate: Unselected agent needing uninstall does not block only target
    # =========================================================================

    def test_agents_gate_with_only_ignores_unselected_agent_uninstall(self):
        self._write_config(
            "agents.json",
            {
                "claude-code": {
                    "display_name": "Claude Code",
                    "bin": "claude",
                    "preferred_provider": "npm",
                    "providers": {"npm": {"managed": True, "package": "@anthropic-ai/claude-code"}},
                },
                "qoder": {
                    "display_name": "Qoder",
                    "bin": "qoder",
                    "preferred_provider": "npm",
                    "providers": {"npm": {"managed": True, "package": "qoder"}},
                },
            },
        )
        self._write_config(
            "manager.json",
            {
                "tools": {}, "skills": {}, "mcp": {},
                "agents": {
                    "claude-code": {"enabled": True, "auto_install": False, "auto_update": False},
                    "qoder": {"enabled": False, "auto_install": False, "auto_update": False},
                },
                "browser_us_sync": {"enabled": False},
            },
        )

        apply_res = self._payload("agents:apply", "--only", "agents:claude-code", "--json", "--offline", "--dry-run")
        self.assertTrue(apply_res["ok"])

    # =========================================================================
    # 8. Cross-domain and Unknown Scopes: Fail before writing with exit code 2
    # =========================================================================

    def test_scope_validation_failures_zero_writes(self):
        target = self.home / ".claude.json"
        target.write_text("{\"initial\": true}\n", encoding="utf-8")
        self._write_config(
            "mcp.json",
            {
                "servers": {"alpha": {"command": "alpha-cmd", "args": [], "env": {}, "agents": ["claude-code"]}},
                "agent_paths": {"claude-code": str(target)},
            },
        )
        self._write_config("manager.json", {"agents": {}, "tools": {}, "skills": {}, "mcp": {"alpha": {"enabled": True}}, "browser_us_sync": {"enabled": False}})

        # Cross-domain scope
        res1 = self._run("mcp:apply", "--only", "tools:some-tool", "--json")
        self.assertEqual(res1.returncode, 2)
        payload1 = json.loads(res1.stdout)
        self.assertEqual(payload1["errors"][0]["code"], "CROSS_DOMAIN_SCOPE")

        # Unknown element without evidence
        res2 = self._run("mcp:apply", "--only", "mcp:random-nonexistent", "--json")
        self.assertEqual(res2.returncode, 2)
        payload2 = json.loads(res2.stdout)
        self.assertEqual(payload2["errors"][0]["code"], "USAGE_ERROR")

        # Zero writes check
        self.assertEqual(target.read_text(encoding="utf-8"), "{\"initial\": true}\n")

    # =========================================================================
    # 9. Dry-Run with Scope: Zero writes
    # =========================================================================

    def test_dry_run_with_only_writes_nothing(self):
        target = self.home / ".claude.json"
        self._write_config(
            "mcp.json",
            {
                "servers": {"alpha": {"command": "alpha-cmd", "args": [], "env": {}, "agents": ["claude-code"]}},
                "agent_paths": {"claude-code": str(target)},
            },
        )
        self._write_config("manager.json", {"agents": {}, "tools": {}, "skills": {}, "mcp": {"alpha": {"enabled": True}}, "browser_us_sync": {"enabled": False}})

        res = self._payload("mcp:apply", "--only", "mcp:alpha", "--dry-run", "--json")
        self.assertTrue(res["ok"])
        self.assertTrue(res["data"]["dry_run"])
        self.assertFalse(target.exists(), "dry-run must write nothing")

    # =========================================================================
    # 10. Multi-Agent Repo: Single skill + single agent apply does not fail matrix validation
    # =========================================================================

    def test_skills_scope_single_skill_single_agent_with_multi_agent_repo_matrix(self):
        repo_base = self.repo / "agent-skill" / "repositories" / "my-repo"
        repo_base.mkdir(parents=True, exist_ok=True)
        (repo_base / ".git").mkdir(exist_ok=True)
        (repo_base / "skill-a").mkdir()
        (repo_base / "skill-a" / "SKILL.md").write_text("---\nname: skill-a\n---\n", encoding="utf-8")
        (repo_base / "skill-b").mkdir()
        (repo_base / "skill-b" / "SKILL.md").write_text("---\nname: skill-b\n---\n", encoding="utf-8")

        claude_skills = self.home / ".claude" / "skills"
        claude_skills.mkdir(parents=True, exist_ok=True)
        codex_skills = self.home / ".codex" / "skills"
        codex_skills.mkdir(parents=True, exist_ok=True)

        self._write_config(
            "skills.json",
            {
                "repositories": {
                    "my-repo": {
                        "url": "https://example.invalid/my-repo.git",
                        "agents": ["claude-code", "codex"],
                        "skills": {
                            "skill-a": {"agents": {"claude-code": True, "codex": True}},
                            "skill-b": {"agents": {"claude-code": True, "codex": True}},
                        },
                    }
                },
                "agents": {
                    "claude-code": str(claude_skills),
                    "codex": str(codex_skills),
                },
            },
        )
        self._write_config(
            "manager.json",
            {
                "agents": {}, "tools": {}, "mcp": {},
                "skills": {"my-repo": {"enabled": True}},
                "browser_us_sync": {"enabled": False},
            },
        )

        apply_res = self._payload(
            "skills:apply", "--only", "skill:my-repo/skill-a", "--agent", "codex", "--offline", "--json"
        )
        self.assertTrue(apply_res["ok"])
        self.assertTrue((codex_skills / "skill-a").exists())
        self.assertFalse((codex_skills / "skill-b").exists())
        self.assertFalse((claude_skills / "skill-a").exists(), "claude-code must not be touched")
        self.assertFalse((claude_skills / "skill-b").exists(), "claude-code must not be touched")

    # =========================================================================
    # 11. Snapshot Restore: Scoped apply newly created link is unlinked, unselected untouched
    # =========================================================================

    def test_skills_scope_snapshot_restore_reverts_newly_created_link(self):
        repo_base = self.repo / "agent-skill" / "repositories" / "my-repo"
        repo_base.mkdir(parents=True, exist_ok=True)
        (repo_base / ".git").mkdir(exist_ok=True)
        (repo_base / "skill-a").mkdir()
        (repo_base / "skill-a" / "SKILL.md").write_text("---\nname: skill-a\n---\n", encoding="utf-8")
        (repo_base / "skill-b").mkdir()
        (repo_base / "skill-b" / "SKILL.md").write_text("---\nname: skill-b\n---\n", encoding="utf-8")

        codex_skills = self.home / ".codex" / "skills"
        codex_skills.mkdir(parents=True, exist_ok=True)

        # Pre-seed skill-b as already linked
        os.symlink(str(repo_base / "skill-b"), str(codex_skills / "skill-b"))
        self.assertTrue((codex_skills / "skill-b").exists())
        self.assertFalse((codex_skills / "skill-a").exists())

        self._write_config(
            "skills.json",
            {
                "repositories": {
                    "my-repo": {
                        "url": "https://example.invalid/my-repo.git",
                        "agents": ["codex"],
                    }
                },
                "agents": {"codex": str(codex_skills)},
            },
        )
        self._write_config(
            "manager.json",
            {
                "agents": {}, "tools": {}, "mcp": {},
                "skills": {"my-repo": {"enabled": True}},
                "browser_us_sync": {"enabled": False},
            },
        )

        # Apply only skill-a: snapshot should only cover skill-a (existed: False)
        apply_res = self._payload("skills:apply", "--only", "skill:my-repo/skill-a", "--agent", "codex", "--offline", "--json")
        self.assertTrue(apply_res["ok"])
        run_id = apply_res["run_id"]
        self.assertTrue((codex_skills / "skill-a").exists(), "skill-a link should have been created")
        self.assertTrue((codex_skills / "skill-b").exists(), "skill-b link should still exist")

        # Restore snapshot
        restore_res = self._payload("snapshots:restore", "--run", run_id, "--json")
        self.assertTrue(restore_res["ok"])

        # skill-a was created by this run and must be reverted to non-existent
        self.assertFalse((codex_skills / "skill-a").exists(), "skill-a link must be unlinked after restore")
        # skill-b was untouched by the scoped apply and must still exist
        self.assertTrue((codex_skills / "skill-b").exists(), "skill-b link must remain untouched")

    # =========================================================================
    # 12. Invalid Definition Scope Fail-Close: Zero snapshots & zero writes
    # =========================================================================

    def test_invalid_definition_cannot_bypass_scope_validation(self):
        target = self.home / ".claude.json"
        target.write_text("{\"intact\": true}\n", encoding="utf-8")
        # Invalid definition: command is null
        self._write_config(
            "mcp.json",
            {
                "servers": {"alpha": {"command": None, "args": [], "env": {}, "agents": ["claude-code"]}},
                "agent_paths": {"claude-code": str(target)},
            },
        )
        self._write_config("manager.json", {"agents": {}, "tools": {}, "skills": {}, "mcp": {"alpha": {"enabled": True}}, "browser_us_sync": {"enabled": False}})

        res = self._run("mcp:apply", "--only", "mcp:alpha", "--offline", "--json")
        self.assertEqual(res.returncode, 2, "Must fail with exit code 2")
        payload = json.loads(res.stdout)
        self.assertEqual(payload["errors"][0]["code"], "CONFIG_INVALID")

        # Zero writes check
        self.assertEqual(target.read_text(encoding="utf-8"), "{\"intact\": true}\n")

        # Zero snapshots check: snapshot_dir should not have any snapshots
        snapshots = list(self.snapshot_dir.glob("*")) if self.snapshot_dir.exists() else []
        self.assertEqual(len(snapshots), 0, "No snapshots should have been created on failed scope validation")

    # =========================================================================
    # 13. Unselected Agent File: Neither rewritten nor snapshotted
    # =========================================================================

    def test_unselected_agent_file_is_not_rewritten_or_snapshotted(self):
        target_a = self.home / ".agent-a.json"
        target_b = self.home / ".agent-b.json"

        # Unique format in target_b that would change if re-serialized
        raw_b_content = "{\n  \"mcpServers\":   {\n    \"beta\": {\n      \"command\": \"beta-cmd\"\n    }\n  }\n}\n"
        target_b.write_text(raw_b_content, encoding="utf-8")

        self._write_config(
            "mcp.json",
            {
                "servers": {
                    "alpha": {"command": "alpha-cmd", "args": ["--arg-a"], "env": {}, "agents": ["agent-a"]},
                    "beta": {"command": "beta-cmd", "args": [], "env": {}, "agents": ["agent-b"]},
                },
                "agent_paths": {
                    "agent-a": str(target_a),
                    "agent-b": str(target_b),
                },
            },
        )
        self._write_config(
            "manager.json",
            {
                "agents": {}, "tools": {}, "skills": {},
                "mcp": {"alpha": {"enabled": True}, "beta": {"enabled": True}},
                "browser_us_sync": {"enabled": False},
            },
        )

        apply_res = self._payload("mcp:apply", "--only", "mcp:alpha", "--offline", "--json")
        self.assertTrue(apply_res["ok"])

        # Agent A's file was created
        self.assertTrue(target_a.exists())

        # Agent B's file bytes are 100% UNCHANGED
        self.assertEqual(target_b.read_text(encoding="utf-8"), raw_b_content, "Unselected Agent B file must not be rewritten")

        # Snapshot manifest only contains target_a, NOT target_b
        snap_info = apply_res["data"]["snapshot"]
        self.assertIsNotNone(snap_info)
        self.assertEqual(snap_info["file_count"], 1, "Snapshot must only contain target_a")
        manifest_path = Path(snap_info["path"]) / "manifest.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        snap_paths = [f["path"] for f in manifest["files"]]
        self.assertIn(str(target_a), snap_paths)
        self.assertNotIn(str(target_b), snap_paths)

    # =========================================================================
    # Persistent counter-example regressions (Inbox 126-130 / 138-141 / 147)
    # =========================================================================

    def test_selected_dry_run_does_not_create_missing_target_directory(self):
        target = self.home / "YOUR_MISSING_DIRECTORY" / "YOUR_AGENT.json"
        self._write_config("manager.json", {
            "agents": {}, "tools": {}, "skills": {},
            "mcp": {"YOUR_MCP_A": {"enabled": True}},
            "browser_us_sync": {"enabled": False},
        })
        self._write_config("mcp.json", {
            "servers": {
                "YOUR_MCP_A": {"command": "echo", "args": [], "env": {}, "agents": ["YOUR_AGENT"]}
            },
            "agent_paths": {"YOUR_AGENT": str(target)},
        })
        self._payload("mcp:apply", "--json", "--offline", "--dry-run", "--only", "mcp:YOUR_MCP_A")
        self.assertFalse(target.parent.exists(), "首次配置的演练也不得创建目标目录")
        self.assertFalse(self.snapshot_dir.exists())

    def test_selected_mcp_does_not_pull_shared_secrets(self):
        import shlex
        target = self.home / "YOUR_AGENT.json"
        target.write_text(json.dumps({"mcpServers": {}}) + "\n")
        self._write_config("manager.json", {
            "agents": {}, "tools": {}, "skills": {},
            "mcp": {"YOUR_MCP_A": {"enabled": True}},
            "browser_us_sync": {"enabled": False},
        })
        self._write_config("mcp.json", {
            "servers": {
                "YOUR_MCP_A": {"command": "echo", "args": [], "env": {}, "agents": ["YOUR_AGENT"]}
            },
            "agent_paths": {"YOUR_AGENT": str(target)},
        })
        for repo in (self.repo / "secrets", self.repo / "secrets" / "YOUR_UNRELATED_REPO"):
            (repo / ".git").mkdir(parents=True, exist_ok=True)
        calls = self.temp_dir / "YOUR_GIT_PULLS.log"
        git = self.bindir / "git"
        git.write_text("#!/bin/sh\nif [ \"$1\" = pull ]; then printf '%s\\n' pull >> " + shlex.quote(str(calls)) + "; fi\nexit 0\n")
        git.chmod(0o755)
        env = {"PATH": str(self.bindir) + os.pathsep + os.environ["PATH"]}
        self._payload("mcp:apply", "--json", "--only", "mcp:YOUR_MCP_A", env_extra=env)
        pulls = calls.read_text().splitlines() if calls.exists() else []
        self.assertEqual(pulls, [], "单项 MCP 操作不应拉取共享凭证及其无关子仓库")

    def test_mcp_runtime_preparation_is_declared_and_runs_only_selected(self):
        import shlex
        target = self.home / "YOUR_AGENT.json"
        target.write_text(json.dumps({"mcpServers": {}}) + "\n")
        calls = self.temp_dir / "YOUR_RUNTIME_CALLS.log"
        definitions = {
            "servers": {
                "YOUR_MCP_A": {"command": "echo", "args": [], "env": {}, "agents": ["YOUR_AGENT"]},
                "YOUR_MCP_B": {"command": "echo", "args": [], "env": {}, "agents": ["YOUR_AGENT"]},
            },
            "agent_paths": {"YOUR_AGENT": str(target)},
        }
        for name in ("YOUR_MCP_A", "YOUR_MCP_B"):
            repo = self.repo / "agent-mcp" / name
            (repo / ".git").mkdir(parents=True, exist_ok=True)
            (repo / "package.json").write_text(json.dumps({"name": "your-package", "scripts": {"build": "true"}}))
            definitions["servers"][name].update({
                "url": "https://example.invalid/" + name + ".git",
                "setup": "printf '%s\\n' " + shlex.quote(name + ":setup") + " >> " + shlex.quote(str(calls)),
            })
        self._write_config("manager.json", {
            "agents": {}, "tools": {}, "skills": {},
            "mcp": {"YOUR_MCP_A": {"enabled": True}, "YOUR_MCP_B": {"enabled": True}},
            "browser_us_sync": {"enabled": False},
        })
        self._write_config("mcp.json", definitions)
        git = self.bindir / "git"
        git.write_text("#!/bin/sh\nexit 0\n")
        git.chmod(0o755)
        npm = self.bindir / "npm"
        npm.write_text("#!/bin/sh\nprintf '%s\\n' \"${PWD##*/}:npm $*\" >> " + shlex.quote(str(calls)) + "\n")
        npm.chmod(0o755)
        env = {"PATH": str(self.bindir) + os.pathsep + os.environ["PATH"]}
        plan = self._payload("mcp:plan", "--json", "--only", "mcp:YOUR_MCP_A", env_extra=env)
        network = [a for a in plan["data"]["actions"] if a["risk"] == "network"]
        self.assertTrue(network, "仓库探测、依赖安装与构建不能只预览配置文件修改")
        self.assertEqual({a["element"] for a in network}, {"mcp:YOUR_MCP_A"})
        self._payload("mcp:apply", "--json", "--only", "mcp:YOUR_MCP_A", env_extra=env)
        self.assertEqual(calls.read_text().splitlines(), ["YOUR_MCP_A:setup", "YOUR_MCP_A:npm install", "YOUR_MCP_A:npm run build"])

    def test_toml_prefix_name_is_not_a_selected_server(self):
        target = self.home / "YOUR_AGENT.toml"
        target.write_text(
            '[mcp_servers.YOUR_MCP_A]\ncommand = "YOUR_OLD_COMMAND_A"\nargs = []\n'
            '[mcp_servers.YOUR_MCP_A.env]\n_MANAGED_BY = "agent-manager"\n'
        )
        other = self.home / "YOUR_OTHER_AGENT.toml"
        other.write_text(
            '\n\n[mcp_servers.YOUR_MCP_A_EXTRA]\ncommand = "YOUR_KEEP_COMMAND"\n'
            '[mcp_servers.YOUR_MCP_A_EXTRA.env]\n_MANAGED_BY = "agent-manager"\n\n'
        )
        self._write_config("manager.json", {
            "agents": {}, "tools": {}, "skills": {},
            "mcp": {"YOUR_MCP_A": {"enabled": True}, "YOUR_MCP_A_EXTRA": {"enabled": True}},
            "browser_us_sync": {"enabled": False},
        })
        self._write_config("mcp.json", {
            "servers": {
                "YOUR_MCP_A": {"command": "YOUR_NEW_COMMAND_A", "args": [], "env": {}, "agents": ["YOUR_AGENT"]},
                "YOUR_MCP_A_EXTRA": {"command": "YOUR_KEEP_COMMAND", "args": [], "env": {}, "agents": ["YOUR_OTHER_AGENT"]},
            },
            "agent_paths": {
                "YOUR_AGENT": {"path": str(target), "format": "toml"},
                "YOUR_OTHER_AGENT": {"path": str(other), "format": "toml"},
            },
        })
        before = other.read_bytes()
        result = self._payload("mcp:apply", "--json", "--offline", "--only", "mcp:YOUR_MCP_A")
        self.assertEqual(other.read_bytes(), before, "A 不得按 TOML 字符串前缀命中 A_EXTRA 的独立文件")
        manifest = json.loads((Path(result["data"]["snapshot"]["path"]) / "manifest.json").read_text())
        self.assertNotIn(str(other), {entry["path"] for entry in manifest["files"]})

    def test_removed_collection_cleans_only_its_managed_links(self):
        coll = self.repo / "agent-skill" / "repositories" / "coll"
        legacy = self.repo / "agent-skill" / "repositories" / "legacy"
        (coll / "skill-a").mkdir(parents=True, exist_ok=True)
        (legacy / "skill-x").mkdir(parents=True, exist_ok=True)
        claude = self.home / ".claude" / "skills"
        claude.mkdir(parents=True, exist_ok=True)
        selected = claude / "skill-a"
        other = claude / "skill-x"
        selected.symlink_to(coll / "skill-a")
        other.symlink_to(legacy / "skill-x")
        before = os.readlink(other)
        self._write_config("manager.json", {
            "agents": {}, "tools": {},
            "skills": {"legacy": {"enabled": True}},
            "mcp": {}, "browser_us_sync": {"enabled": False},
        })
        self._write_config("skills.json", {
            "repositories": {
                "legacy": {"url": "https://example.com/legacy.git", "skill_paths": ["skill-x"], "agents": ["claude"]}
            },
            "agents": {"claude": {"path": str(claude)}},
        })
        plan = self._payload("skills:plan", "--json", "--offline", "--only", "skill:coll")
        self.assertIn(str(selected), {a["target"] for a in plan["data"]["actions"]})
        self.assertNotIn(str(other), {a["target"] for a in plan["data"]["actions"]})
        self._payload("skills:apply", "--json", "--offline", "--only", "skill:coll")
        self.assertFalse(os.path.lexists(selected), "已移除集合的受管链接须真正清理")
        self.assertEqual(os.readlink(other), before)
        self.assertTrue(legacy.exists())

    def test_cached_collection_unknown_freshness_previews_network(self):
        coll = self.repo / "agent-skill" / "repositories" / "coll"
        (coll / ".git").mkdir(parents=True, exist_ok=True)
        (coll / "skill-a").mkdir(parents=True, exist_ok=True)
        claude = self.home / ".claude" / "skills"
        claude.mkdir(parents=True, exist_ok=True)
        self._write_config("manager.json", {
            "agents": {}, "tools": {},
            "skills": {"coll": {"enabled": True}},
            "mcp": {}, "browser_us_sync": {"enabled": False},
        })
        self._write_config("skills.json", {
            "repositories": {
                "coll": {"url": "https://example.com/coll.git", "skill_paths": ["skill-a"], "agents": ["claude"]}
            },
            "agents": {"claude": {"path": str(claude)}},
        })
        plan = self._payload("skills:plan", "--json", "--only", "skill:coll")
        network = [a for a in plan["data"]["actions"] if a["risk"] == "network"]
        self.assertTrue(network, "缓存集合 auto_update 默认启用，未知 freshness 的预览仍须声明上游检查")
        self.assertEqual({a["element"] for a in network}, {"skill:coll"})

    def test_selected_alias_snapshot_never_restores_other_rc_file(self):
        self._write_config(
            "shell-aliases.json",
            {"timezone": "UTC", "lang": "C", "aliases": {"ll": "ls -l"}},
        )
        selected = self.home / ".zshrc"
        other = self.home / ".zprofile"
        selected.write_text("# YOUR_SELECTED_OLD_CONTENT\n")
        other.write_text("# YOUR_UNSELECTED_OLD_CONTENT\n")
        result = self._payload("aliases:apply", "--json", "--offline", "--only", "aliases:zshrc")
        self.assertIn("alias ll=", selected.read_text())
        self.assertEqual(other.read_text(), "# YOUR_UNSELECTED_OLD_CONTENT\n")
        snapshot = result["data"]["snapshot"]
        manifest = json.loads((Path(snapshot["path"]) / "manifest.json").read_text())
        self.assertEqual({entry["path"] for entry in manifest["files"]}, {str(selected)})
        other.write_text("# YOUR_UNSELECTED_NEW_CONTENT\n")
        self._payload("snapshots:restore", "--json", "--run", snapshot["run_id"])
        self.assertEqual(selected.read_text(), "# YOUR_SELECTED_OLD_CONTENT\n")
        self.assertEqual(other.read_text(), "# YOUR_UNSELECTED_NEW_CONTENT\n")


if __name__ == "__main__":
    unittest.main()
