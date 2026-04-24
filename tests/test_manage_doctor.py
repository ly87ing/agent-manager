import shutil
import tempfile
import unittest
import json
import os
from pathlib import Path
from unittest import mock

from src.manage_doctor import actual_skill_targets, build_report


ROOT = Path(__file__).resolve().parent.parent


class ManageDoctorReportTests(unittest.TestCase):
    def test_actual_skill_targets_includes_windows_junctions(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "targets"
            junction_path = target_dir / "frontend-skill"
            managed_target = Path(temp_dir) / "managed" / "frontend-skill"
            target_dir.mkdir()
            junction_path.mkdir()
            real_readlink = os.readlink

            with (
                mock.patch.object(
                    os.path,
                    "isjunction",
                    side_effect=lambda path: Path(path) == junction_path,
                    create=True,
                ),
                mock.patch(
                    "src.manage_doctor.os.readlink",
                    side_effect=lambda path: str(managed_target) if Path(path) == junction_path else real_readlink(path),
                ),
            ):
                actual_targets, broken_names = actual_skill_targets(str(target_dir))

            self.assertEqual(
                actual_targets,
                {"frontend-skill": str(managed_target.resolve(strict=False))},
            )
            self.assertEqual(broken_names, set())

    def test_actual_skill_targets_normalizes_windows_substitution_path(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            target_dir = Path(temp_dir) / "targets"
            junction_path = target_dir / "frontend-skill"
            target_dir.mkdir()
            junction_path.mkdir()
            substitution_target = r"\\?\C:\repo\agent-skill\repositories\demo-repo\bundle\frontend-skill"
            normalized_target = r"C:\repo\agent-skill\repositories\demo-repo\bundle\frontend-skill"
            real_readlink = os.readlink

            with (
                mock.patch.object(
                    os.path,
                    "isjunction",
                    side_effect=lambda path: Path(path) == junction_path,
                    create=True,
                ),
                mock.patch(
                    "src.manage_doctor.os.readlink",
                    side_effect=lambda path: substitution_target if Path(path) == junction_path else real_readlink(path),
                ),
            ):
                actual_targets, broken_names = actual_skill_targets(str(target_dir))

            self.assertEqual(
                actual_targets,
                {"frontend-skill": normalized_target},
            )
            self.assertEqual(broken_names, set())

    def test_report_flags_enabled_agent_when_provider_exists_but_runtime_entrypoint_is_missing(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {\n    "claude-code": { "enabled": true, "auto_install": true, "auto_update": true }\n  },\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "agents.json").write_text(
                '{\n  "claude-code": {\n    "display_name": "Claude Code",\n    "bin": "claude",\n    "preferred_provider": "brew",\n    "providers": {\n      "brew": {\n        "install": "brew install --cask claude-code",\n        "upgrade": "brew upgrade --cask claude-code"\n      }\n    }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{"agent_paths": {}, "servers": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{"repositories": {}, "agents": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            with (
                mock.patch("src.manage_doctor.detect_all_providers", return_value=["brew-cask"]),
                mock.patch("src.manage_doctor.is_agent_installed", return_value=False),
            ):
                report = build_report(
                    root_dir=temp_root,
                    config_file=temp_root / "config" / "manager.json",
                    python_cmd="python3",
                )

            self.assertEqual(report["status"], "error")
            agent_names = {entry["name"]: entry for entry in report["sections"]["agents"]}
            self.assertEqual(agent_names["Claude Code"]["status"], "error")
            self.assertIn("运行入口缺失", agent_names["Claude Code"]["message"])

    def test_report_flags_deprecated_provider_as_error(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {\n    "claude-code": { "enabled": true, "auto_install": true, "auto_update": true, "preferred_provider": "native" }\n  },\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "agents.json").write_text(
                '{\n  "claude-code": {\n    "display_name": "Claude Code",\n    "bin": "claude",\n    "require_managed_provider": true,\n    "providers": {\n      "native": { "managed": true, "upgrade": "claude update" },\n      "brew": { "managed": false, "install": "brew install --cask claude-code" }\n    }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{"agent_paths": {}, "servers": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{"repositories": {}, "agents": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            with (
                mock.patch("src.manage_doctor.detect_all_providers", return_value=["brew-cask"]),
                mock.patch("src.manage_doctor.is_agent_installed", return_value=True),
            ):
                report = build_report(
                    root_dir=temp_root,
                    config_file=temp_root / "config" / "manager.json",
                    python_cmd="python3",
                )

            agent_names = {entry["name"]: entry for entry in report["sections"]["agents"]}
            self.assertEqual(report["status"], "error")
            self.assertEqual(agent_names["Claude Code"]["status"], "error")
            self.assertIn("已弃用安装来源", agent_names["Claude Code"]["message"])

    def test_report_flags_system_only_provider_when_managed_provider_is_required(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {\n    "codex": { "enabled": true, "auto_install": true, "auto_update": true, "preferred_provider": "npm" }\n  },\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "agents.json").write_text(
                '{\n  "codex": {\n    "display_name": "Codex",\n    "bin": "codex",\n    "require_managed_provider": true,\n    "providers": {\n      "npm": { "managed": true, "upgrade": "npm install -g @openai/codex@latest" }\n    }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{"agent_paths": {}, "servers": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{"repositories": {}, "agents": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            with (
                mock.patch("src.manage_doctor.detect_all_providers", return_value=["system"]),
                mock.patch("src.manage_doctor.is_agent_installed", return_value=True),
            ):
                report = build_report(
                    root_dir=temp_root,
                    config_file=temp_root / "config" / "manager.json",
                    python_cmd="python3",
                )

            agent_names = {entry["name"]: entry for entry in report["sections"]["agents"]}
            self.assertEqual(report["status"], "error")
            self.assertEqual(agent_names["Codex"]["status"], "error")
            self.assertIn("未纳入 agent-manager 托管", agent_names["Codex"]["message"])

    def test_report_flags_repo_secret_outside_secrets_directory(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            openai_key = "sk-proj-" + "abcdefghijklmnopqrstuvwxyz12"
            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {},\n  "servers": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")
            (temp_root / "leak.txt").write_text(
                f'OPENAI_API_KEY="{openai_key}"\n',
                encoding="utf-8",
            )

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            security_entries = {entry["name"]: entry for entry in report["sections"]["security"]}
            self.assertIn("leak.txt", security_entries)
            self.assertIn("openai_like_key", security_entries["leak.txt"]["message"])

    def test_report_repo_secret_scan_ignores_placeholder_examples(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {},\n  "servers": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")
            (temp_root / "notes.md").write_text(
                'AUGMENT_SESSION_AUTH=\'{"accessToken":"ace_...","tenantURL":"...","scopes":[]}\'\n',
                encoding="utf-8",
            )

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertNotEqual(report["status"], "error")
            security_entries = {entry["name"]: entry for entry in report["sections"]["security"]}
            self.assertEqual(security_entries["tracked repo files"]["status"], "ok")

    def test_report_requires_toml_parser_for_active_toml_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "codex": { "path": "%s/.codex/config.toml", "format": "toml" }\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp"],\n      "agents": ["codex"]\n    }\n  }\n}\n'
                % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            with mock.patch("src.manage_doctor.load_tomllib", side_effect=ModuleNotFoundError("tomli missing")):
                report = build_report(
                    root_dir=temp_root,
                    config_file=temp_root / "config" / "manager.json",
                    python_cmd="python3",
                )

            self.assertEqual(report["status"], "error")
            self.assertEqual(report["summary"]["errors"], 1)
            runtime_names = {entry["name"]: entry for entry in report["sections"]["runtime"]}
            self.assertIn("TOML parser", runtime_names)
            self.assertEqual(runtime_names["TOML parser"]["status"], "error")
            self.assertIn("tomli", runtime_names["TOML parser"]["message"])

    def test_report_flags_existing_mcp_file_with_missing_managed_entries(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            codex_config = temp_home / ".codex" / "config.toml"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            codex_config.parent.mkdir(parents=True)
            codex_config.write_text("[mcp_servers.user-owned]\ncommand = \"echo\"\n", encoding="utf-8")

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "codex": { "path": "%s", "format": "toml" }\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp"],\n      "agents": ["codex"]\n    }\n  }\n}\n'
                % codex_config,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            mcp_names = {entry["name"]: entry for entry in report["sections"]["mcp"]}
            self.assertIn("Codex", mcp_names)
            self.assertEqual(mcp_names["Codex"]["status"], "error")
            self.assertIn("期望 1 个托管服务", mcp_names["Codex"]["message"])

    def test_report_flags_kimi_cli_json_target_with_missing_managed_entries(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            kimi_config = temp_home / ".kimi" / "mcp.json"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            kimi_config.parent.mkdir(parents=True)
            kimi_config.write_text(
                json.dumps({"mcpServers": {"user-owned": {"command": "echo", "args": []}}}),
                encoding="utf-8",
            )

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "kimi-cli": { "path": "%s", "format": "json" }\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp"],\n      "agents": ["kimi-cli"]\n    }\n  }\n}\n'
                % kimi_config,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            mcp_names = {entry["name"]: entry for entry in report["sections"]["mcp"]}
            self.assertIn("Kimi Code CLI", mcp_names)
            self.assertEqual(mcp_names["Kimi Code CLI"]["status"], "error")
            self.assertIn("期望 1 个托管服务", mcp_names["Kimi Code CLI"]["message"])

    def test_rules_audit_marks_kimi_target_unmaterialized_when_installed(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            target_path = temp_root / ".kimi" / "AGENTS.md"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))

            (temp_root / "config" / "manager.json").write_text(
                (
                    '{\n  "agents": {\n    "kimi-cli": { "enabled": true, "auto_install": false, "auto_update": false }\n  },'
                    '\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n'
                ),
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{"agent_paths": {}, "servers": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{"repositories": {}, "agents": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text(
                "kimi_global|global|rules/dist/kimi/AGENTS.md|{{REPO_ROOT}}/.kimi/AGENTS.md\n",
                encoding="utf-8",
            )
            target_path.unlink(missing_ok=True)

            with mock.patch(
                "src.manage_doctor.is_agent_installed",
                side_effect=lambda agent_id, _definition: agent_id == "kimi-cli",
            ):
                report = build_report(
                    root_dir=temp_root,
                    config_file=temp_root / "config" / "manager.json",
                    python_cmd="python3",
                )

            rules_names = {entry["name"]: entry for entry in report["sections"]["rules"]}
            self.assertEqual(rules_names["kimi_global"]["status"], "warning")
            self.assertEqual(rules_names["kimi_global"]["message"], "尚未分发")

    def test_rules_audit_marks_kimi_target_unmaterialized_when_uninstalled(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            target_path = temp_root / ".kimi" / "AGENTS.md"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))

            (temp_root / "config" / "manager.json").write_text(
                (
                    '{\n  "agents": {\n    "kimi-cli": { "enabled": true, "auto_install": false, "auto_update": false }\n  },'
                    '\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n'
                ),
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{"agent_paths": {}, "servers": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{"repositories": {}, "agents": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text(
                "kimi_global|global|rules/dist/kimi/AGENTS.md|{{REPO_ROOT}}/.kimi/AGENTS.md\n",
                encoding="utf-8",
            )
            target_path.unlink(missing_ok=True)

            with mock.patch(
                "src.manage_doctor.is_agent_installed",
                side_effect=lambda agent_id, _definition: False if agent_id == "kimi-cli" else True,
            ):
                report = build_report(
                    root_dir=temp_root,
                    config_file=temp_root / "config" / "manager.json",
                    python_cmd="python3",
                )

            rules_names = {entry["name"]: entry for entry in report["sections"]["rules"]}
            self.assertEqual(rules_names["kimi_global"]["status"], "warning")
            self.assertEqual(rules_names["kimi_global"]["message"], "尚未分发")

    def test_report_ignores_foreign_managed_by_marker_in_toml_mcp_file(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            codex_config = temp_home / ".codex" / "config.toml"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            codex_config.parent.mkdir(parents=True)
            codex_config.write_text(
                "\n".join(
                    [
                        "[mcp_servers.user-owned]",
                        'command = "echo"',
                        "args = []",
                        "[mcp_servers.user-owned.env]",
                        '_MANAGED_BY = "someone-else"',
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "codex": { "path": "%s", "format": "toml" }\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp"],\n      "agents": ["codex"]\n    }\n  }\n}\n'
                % codex_config,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            mcp_names = {entry["name"]: entry for entry in report["sections"]["mcp"]}
            self.assertEqual(mcp_names["Codex"]["status"], "error")
            self.assertIn("实际 0 个", mcp_names["Codex"]["message"])

    def test_report_ignores_foreign_managed_by_marker_in_json_mcp_file(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            cursor_config = Path(temp_dir) / "cursor" / "mcp.json"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            cursor_config.parent.mkdir(parents=True)
            cursor_config.write_text(
                json.dumps(
                    {
                        "mcpServers": {
                            "user-owned": {
                                "command": "echo",
                                "args": [],
                                "env": {"_MANAGED_BY": "someone-else"},
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "cursor": { "path": "%s", "format": "json" }\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp"],\n      "agents": ["cursor"]\n    }\n  }\n}\n'
                % cursor_config,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            mcp_names = {entry["name"]: entry for entry in report["sections"]["mcp"]}
            self.assertEqual(mcp_names["Cursor IDE"]["status"], "error")
            self.assertIn("实际 0 个", mcp_names["Cursor IDE"]["message"])

    def test_report_flags_wrong_managed_mcp_name_even_when_count_matches(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            codex_config = temp_home / ".codex" / "config.toml"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            codex_config.parent.mkdir(parents=True)
            codex_config.write_text(
                "\n".join(
                    [
                        "[mcp_servers.bad-mcp]",
                        'command = "echo"',
                        "args = []",
                        "[mcp_servers.bad-mcp.env]",
                        '_MANAGED_BY = "agent-manager"',
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "codex": { "path": "%s", "format": "toml" }\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp"],\n      "agents": ["codex"]\n    }\n  }\n}\n'
                % codex_config,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            mcp_names = {entry["name"]: entry for entry in report["sections"]["mcp"]}
            self.assertEqual(mcp_names["Codex"]["status"], "error")
            self.assertIn("augment-context-engine", mcp_names["Codex"]["message"])
            self.assertIn("bad-mcp", mcp_names["Codex"]["message"])

    def test_report_flags_missing_managed_skill_even_when_unmanaged_link_count_matches(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            target_dir = Path(temp_dir) / "targets" / "claude"
            managed_repo = temp_root / "agent-skill" / "repositories" / "demo-repo"
            managed_skill = managed_repo / "bundle" / "managed-skill"
            external_skill = Path(temp_dir) / "external" / "user-skill"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            target_dir.mkdir(parents=True)
            managed_skill.mkdir(parents=True)
            (managed_repo / ".git").mkdir()
            (managed_repo / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            (managed_skill / "SKILL.md").write_text(
                "---\nname: managed-skill\ndescription: Managed\n---\n\n# Managed\n",
                encoding="utf-8",
            )
            external_skill.mkdir(parents=True)
            (target_dir / "user-skill").symlink_to(external_skill)

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {\n    "demo-repo": { "enabled": true }\n  },\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                json.dumps(
                    {
                        "repositories": {
                            "demo-repo": {
                                "url": "https://example.invalid/repo.git",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(target_dir),
                        },
                    }
                ),
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{"servers": {}, "agent_paths": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            skill_names = {entry["name"]: entry for entry in report["sections"]["skills"]}
            self.assertEqual(skill_names["Claude Code"]["status"], "error")
            self.assertIn("managed-skill", skill_names["Claude Code"]["message"])

    def test_report_ignores_extra_unmanaged_skill_link_when_managed_targets_match(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            target_dir = Path(temp_dir) / "targets" / "claude"
            managed_repo = temp_root / "agent-skill" / "repositories" / "demo-repo"
            managed_skill = managed_repo / "bundle" / "managed-skill"
            external_skill = Path(temp_dir) / "external" / "user-skill"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            target_dir.mkdir(parents=True)
            managed_skill.mkdir(parents=True)
            (managed_repo / ".git").mkdir()
            (managed_repo / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            (managed_skill / "SKILL.md").write_text(
                "---\nname: managed-skill\ndescription: Managed\n---\n\n# Managed\n",
                encoding="utf-8",
            )
            external_skill.mkdir(parents=True)
            (target_dir / "managed-skill").symlink_to(managed_skill)
            (target_dir / "user-skill").symlink_to(external_skill)

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {\n    "demo-repo": { "enabled": true }\n  },\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                json.dumps(
                    {
                        "repositories": {
                            "demo-repo": {
                                "url": "https://example.invalid/repo.git",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(target_dir),
                        },
                    }
                ),
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{"servers": {}, "agent_paths": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            skill_names = {entry["name"]: entry for entry in report["sections"]["skills"]}
            self.assertEqual(skill_names["Claude Code"]["status"], "ok")
            self.assertEqual(skill_names["Claude Code"]["message"], "1 个受管链接")

    def test_report_flags_extra_managed_skill_link_not_declared_in_policy(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            target_dir = Path(temp_dir) / "targets" / "claude"
            managed_repo = temp_root / "agent-skill" / "repositories" / "demo-repo"
            managed_skill = managed_repo / "bundle" / "managed-skill"
            extra_repo = temp_root / "agent-skill" / "repositories" / "old-repo"
            extra_skill = extra_repo / "bundle" / "other-skill"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            target_dir.mkdir(parents=True)
            managed_skill.mkdir(parents=True)
            extra_skill.mkdir(parents=True)
            (managed_repo / ".git").mkdir()
            (managed_repo / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            (extra_repo / ".git").mkdir()
            (extra_repo / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            (managed_skill / "SKILL.md").write_text(
                "---\nname: managed-skill\ndescription: Managed\n---\n\n# Managed\n",
                encoding="utf-8",
            )
            (extra_skill / "SKILL.md").write_text(
                "---\nname: other-skill\ndescription: Old\n---\n\n# Old\n",
                encoding="utf-8",
            )
            (target_dir / "managed-skill").symlink_to(managed_skill)
            (target_dir / "other-skill").symlink_to(extra_skill)

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {\n    "demo-repo": { "enabled": true }\n  },\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                json.dumps(
                    {
                        "repositories": {
                            "demo-repo": {
                                "url": "https://example.invalid/repo.git",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(target_dir),
                        },
                    }
                ),
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{"servers": {}, "agent_paths": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            skill_names = {entry["name"]: entry for entry in report["sections"]["skills"]}
            self.assertEqual(skill_names["Claude Code"]["status"], "error")
            self.assertIn("other-skill", skill_names["Claude Code"]["message"])

    def test_report_flags_broken_skill_link_even_when_managed_targets_match(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            target_dir = Path(temp_dir) / "targets" / "claude"
            managed_repo = temp_root / "agent-skill" / "repositories" / "demo-repo"
            managed_skill = managed_repo / "bundle" / "managed-skill"
            missing_skill = Path(temp_dir) / "external" / "missing-skill"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            target_dir.mkdir(parents=True)
            managed_skill.mkdir(parents=True)
            (managed_repo / ".git").mkdir()
            (managed_repo / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
            (managed_skill / "SKILL.md").write_text(
                "---\nname: managed-skill\ndescription: Managed\n---\n\n# Managed\n",
                encoding="utf-8",
            )
            (target_dir / "managed-skill").symlink_to(managed_skill)
            (target_dir / "dead-skill").symlink_to(missing_skill)

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {\n    "demo-repo": { "enabled": true }\n  },\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                json.dumps(
                    {
                        "repositories": {
                            "demo-repo": {
                                "url": "https://example.invalid/repo.git",
                                "agents": ["claude-code"],
                            }
                        },
                        "agents": {
                            "claude-code": str(target_dir),
                        },
                    }
                ),
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{"servers": {}, "agent_paths": {}}',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            skill_names = {entry["name"]: entry for entry in report["sections"]["skills"]}
            self.assertEqual(skill_names["Claude Code"]["status"], "error")
            self.assertIn("dead-skill", skill_names["Claude Code"]["message"])

    def test_report_flags_managed_mcp_content_drift_for_expected_server(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            codex_config = temp_home / ".codex" / "config.toml"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            codex_config.parent.mkdir(parents=True)
            codex_config.write_text(
                "\n".join(
                    [
                        "[mcp_servers.augment-context-engine]",
                        'command = "echo"',
                        'args = ["--wrong"]',
                        "[mcp_servers.augment-context-engine.env]",
                        '_MANAGED_BY = "agent-manager"',
                        'AUGMENT_SESSION_AUTH = "resolved-token"',
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "codex": { "path": "%s", "format": "toml" }\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp", "--mcp-auto-workspace"],\n      "env": { "AUGMENT_SESSION_AUTH": "${AUGMENT_SESSION_AUTH}" },\n      "agents": ["codex"]\n    }\n  }\n}\n'
                % codex_config,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            mcp_names = {entry["name"]: entry for entry in report["sections"]["mcp"]}
            self.assertEqual(mcp_names["Codex"]["status"], "error")
            self.assertIn("augment-context-engine", mcp_names["Codex"]["message"])
            self.assertIn("配置漂移", mcp_names["Codex"]["message"])

    def test_report_accepts_resolved_repo_path_for_repo_backed_managed_mcp_server(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            cursor_config = Path(temp_dir) / "cursor" / "mcp.json"
            repo_path = temp_root / "agent-mcp" / "demo"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            cursor_config.parent.mkdir(parents=True)
            repo_path.mkdir(parents=True)
            (repo_path / ".git").mkdir()
            cursor_config.write_text(
                json.dumps(
                    {
                        "mcpServers": {
                            "demo": {
                                "command": "node",
                                "args": [f"{repo_path.resolve()}/server.js"],
                                "env": {"_MANAGED_BY": "agent-manager"},
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "demo": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                json.dumps(
                    {
                        "agent_paths": {
                            "cursor": {"path": str(cursor_config), "format": "json"},
                        },
                        "servers": {
                            "demo": {
                                "url": "https://example.invalid/repo.git",
                                "command": "node",
                                "args": ["{repo_path}/server.js"],
                                "env": {},
                                "agents": ["cursor"],
                            }
                        },
                    }
                ),
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "healthy")
            mcp_names = {entry["name"]: entry for entry in report["sections"]["mcp"]}
            self.assertEqual(mcp_names["Cursor IDE"]["status"], "ok")

    def test_report_accepts_resolved_secret_for_managed_mcp_server(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            codex_config = temp_home / ".codex" / "config.toml"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            codex_config.parent.mkdir(parents=True)
            codex_config.write_text(
                "\n".join(
                    [
                        "[mcp_servers.augment-context-engine]",
                        'command = "auggie"',
                        'args = ["--mcp", "--mcp-auto-workspace"]',
                        "[mcp_servers.augment-context-engine.env]",
                        '_MANAGED_BY = "agent-manager"',
                        'AUGMENT_SESSION_AUTH = "resolved-token"',
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "codex": { "path": "%s", "format": "toml" }\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp", "--mcp-auto-workspace"],\n      "env": { "AUGMENT_SESSION_AUTH": "${AUGMENT_SESSION_AUTH}" },\n      "agents": ["codex"]\n    }\n  }\n}\n'
                % codex_config,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "healthy")
            mcp_names = {entry["name"]: entry for entry in report["sections"]["mcp"]}
            self.assertEqual(mcp_names["Codex"]["status"], "ok")

    def test_report_flags_opencode_config_using_wrong_mcpservers_key(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            opencode_config = Path(temp_dir) / "opencode.json"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            opencode_config.write_text(
                json.dumps(
                    {
                        "mcpServers": {
                            "demo": {
                                "command": "echo",
                                "args": [],
                                "env": {"_MANAGED_BY": "agent-manager"},
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "demo": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                json.dumps(
                    {
                        "agent_paths": {
                            "opencode": {"path": str(opencode_config), "format": "json"},
                        },
                        "servers": {
                            "demo": {
                                "command": "echo",
                                "args": [],
                                "env": {},
                                "agents": ["opencode"],
                            }
                        },
                    }
                ),
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            mcp_names = {entry["name"]: entry for entry in report["sections"]["mcp"]}
            self.assertEqual(mcp_names["OpenCode"]["status"], "error")
            self.assertIn("实际 0 个", mcp_names["OpenCode"]["message"])

    def test_report_flags_unresolved_placeholder_in_managed_mcp_server(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            codex_config = temp_home / ".codex" / "config.toml"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            codex_config.parent.mkdir(parents=True)
            codex_config.write_text(
                "\n".join(
                    [
                        "[mcp_servers.augment-context-engine]",
                        'command = "auggie"',
                        'args = ["--mcp", "--mcp-auto-workspace"]',
                        "[mcp_servers.augment-context-engine.env]",
                        '_MANAGED_BY = "agent-manager"',
                        'AUGMENT_SESSION_AUTH = "${AUGMENT_SESSION_AUTH}"',
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "codex": { "path": "%s", "format": "toml" }\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp", "--mcp-auto-workspace"],\n      "env": { "AUGMENT_SESSION_AUTH": "${AUGMENT_SESSION_AUTH}" },\n      "agents": ["codex"]\n    }\n  }\n}\n'
                % codex_config,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "error")
            mcp_names = {entry["name"]: entry for entry in report["sections"]["mcp"]}
            self.assertEqual(mcp_names["Codex"]["status"], "error")
            self.assertIn("配置漂移", mcp_names["Codex"]["message"])

    def test_report_accepts_literal_mcp_value_containing_placeholder_substring(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            codex_config = temp_home / ".codex" / "config.toml"
            literal_value = "prefix-${USER}-suffix"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            codex_config.parent.mkdir(parents=True)
            codex_config.write_text(
                "\n".join(
                    [
                        "[mcp_servers.augment-context-engine]",
                        'command = "auggie"',
                        'args = ["--mcp"]',
                        "[mcp_servers.augment-context-engine.env]",
                        '_MANAGED_BY = "agent-manager"',
                        f'PROMPT = "{literal_value}"',
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                json.dumps(
                    {
                        "agent_paths": {
                            "codex": {"path": str(codex_config), "format": "toml"}
                        },
                        "servers": {
                            "augment-context-engine": {
                                "command": "auggie",
                                "args": ["--mcp"],
                                "env": {"PROMPT": literal_value},
                                "agents": ["codex"],
                            }
                        },
                    }
                ),
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            report = build_report(
                root_dir=temp_root,
                config_file=temp_root / "config" / "manager.json",
                python_cmd="python3",
            )

            self.assertEqual(report["status"], "healthy")
            mcp_names = {entry["name"]: entry for entry in report["sections"]["mcp"]}
            self.assertEqual(mcp_names["Codex"]["status"], "ok")
