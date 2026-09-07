import shutil
import tempfile
import unittest
import json
import os
from pathlib import Path
from unittest import mock

from src.manage_doctor import (
    actual_skill_targets,
    build_report,
    expected_managed_mcp_servers,
    render_text,
)


ROOT = Path(__file__).resolve().parent.parent

COPY_IGNORE = shutil.ignore_patterns(
    "repositories", ".git", ".worktrees", "__pycache__", "secrets", ".env", "app", "node_modules", "target", "manager.local.json", "manager.synced.json"
)


def copy_test_repo(src: Path, dst: Path) -> None:
    shutil.copytree(src, dst, ignore=COPY_IGNORE)
    for candidate in (
        src / "engine-rs" / "target" / "release" / "agent-manager-engine",
        src / "engine-rs" / "target" / "debug" / "agent-manager-engine",
    ):
        if candidate.is_file() and os.access(candidate, os.X_OK):
            target_bin = dst / "engine-rs" / "target" / candidate.parent.name / "agent-manager-engine"
            target_bin.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(candidate, target_bin)
            break


class ManageDoctorReportTests(unittest.TestCase):
    def test_expected_mcp_servers_resolve_agent_specific_args(self):
        config = {"mcp": {"serena": {"enabled": True}}}
        mcp_config = {
            "servers": {
                "serena": {
                    "command": "uvx",
                    "args": ["serena", "start-mcp-server"],
                    "extra_args_by_agent": {
                        "claude-code": ["--context", "claude-code"],
                        "codex": ["--context", "codex"],
                    },
                    "env": {},
                    "agents": None,
                }
            }
        }

        codex_servers = expected_managed_mcp_servers(
            ROOT,
            config,
            mcp_config,
            "codex",
        )
        claude_servers = expected_managed_mcp_servers(
            ROOT,
            config,
            mcp_config,
            "claude-code",
        )

        self.assertEqual(
            codex_servers["serena"]["args"],
            ("serena", "start-mcp-server", "--context", "codex"),
        )
        self.assertEqual(
            claude_servers["serena"]["args"],
            ("serena", "start-mcp-server", "--context", "claude-code"),
        )

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
            copy_test_repo(ROOT, temp_root)

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
            copy_test_repo(ROOT, temp_root)

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
            copy_test_repo(ROOT, temp_root)

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

    def test_report_flags_invalid_unsupported_actions_in_doctor_as_error(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {\n    "codex": { "enabled": false, "auto_install": false, "auto_update": false, "preferred_provider": "npm" }\n  },\n  "tools": {},\n  "skills": {},\n  "mcp": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "agents.json").write_text(
                '{\n  "codex": {\n    "display_name": "Codex",\n    "bin": "codex",\n    "preferred_provider": "npm",\n    "providers": {\n      "npm": {\n        "managed": true,\n        "uninstall": "npm uninstall -g @openai/codex",\n        "unsupported_actions": "uninstall"\n      }\n    }\n  }\n}\n',
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
                mock.patch("src.manage_doctor.detect_all_providers", return_value=["native"]),
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
            self.assertIn("unsupported_actions 配置非法", agent_names["Codex"]["message"])
            self.assertEqual(report["summary"]["errors"], 1)
            self.assertEqual(report["summary"]["warnings"], 0)
            self.assertEqual(report["summary"]["warning_categories"]["official_gaps"], 0)
            self.assertNotIn("category", agent_names["Codex"])

    def test_report_flags_repo_secret_outside_secrets_directory(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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

    def test_report_ignores_foreign_managed_by_marker_in_toml_mcp_file(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            codex_config = temp_home / ".codex" / "config.toml"
            copy_test_repo(ROOT, temp_root)
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

    def test_report_flags_wrong_managed_mcp_name_even_when_count_matches(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            codex_config = temp_home / ".codex" / "config.toml"
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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

    def test_report_accepts_resolved_secret_for_managed_mcp_server(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            codex_config = temp_home / ".codex" / "config.toml"
            copy_test_repo(ROOT, temp_root)
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

    def test_report_flags_unresolved_placeholder_in_managed_mcp_server(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            codex_config = temp_home / ".codex" / "config.toml"
            copy_test_repo(ROOT, temp_root)
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
            copy_test_repo(ROOT, temp_root)
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


from src.manage_doctor import collect_skill_discovery_inbox  # noqa: E402


class ManageDoctorSkillMatrixTests(unittest.TestCase):
    """skills v2（提案 §7.1）下 doctor 的矩阵感知期望集与发现收件箱。

    旧式定义必须走升格等价路径：doctor 既有行为逐比特不变。
    """

    @staticmethod
    def _named_skills(*names):
        return {f"bundle/{name}": name for name in names}

    def _write_doctor_fixture(self, temp_root, repo_skills, policy, *, repo_agents=("claude-code", "codex"), seeded_links=None):
        temp_root = Path(temp_root) / "repo"
        copy_test_repo(ROOT, temp_root)
        managed_repo = temp_root / "agent-skill" / "repositories" / "demo-repo"
        (managed_repo / ".git").mkdir(parents=True)
        (managed_repo / ".agent-manager-owned").write_text("true\n", encoding="utf-8")
        for rel_dir, skill_name in repo_skills.items():
            skill_dir = managed_repo / rel_dir
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text(
                f"---\nname: {skill_name}\ndescription: demo\n---\n", encoding="utf-8"
            )
        targets = {
            "claude-code": str(temp_root / "targets" / "claude"),
            "codex": str(temp_root / "targets" / "codex"),
        }
        for agent_name in repo_agents:
            Path(targets[agent_name]).mkdir(parents=True)
        for agent_name, links in (seeded_links or {}).items():
            for link_name, rel_dir in links.items():
                (Path(targets[agent_name]) / link_name).symlink_to(managed_repo / rel_dir)
        config = {"agents": {}, "tools": {}, "skills": {"demo-repo": policy}, "mcp": {}}
        skill_cfg = {
            "repositories": {
                "demo-repo": {
                    "url": "https://example.invalid/repo.git",
                    "agents": list(repo_agents),
                }
            },
            "agents": targets,
        }
        (temp_root / "config" / "manager.json").write_text(json.dumps(config), encoding="utf-8")
        (temp_root / "config" / "skills.json").write_text(json.dumps(skill_cfg), encoding="utf-8")
        (temp_root / "config" / "mcp.json").write_text(
            '{"servers": {}, "agent_paths": {}}', encoding="utf-8"
        )
        (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")
        return temp_root, config, skill_cfg

    def _build(self, temp_root):
        return build_report(
            root_dir=temp_root,
            config_file=temp_root / "config" / "manager.json",
            python_cmd="python3",
        )

    def test_matrix_mode_expected_set_respects_per_agent_switches(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root, _config, _skill_cfg = self._write_doctor_fixture(
                temp_dir,
                self._named_skills("alpha", "beta"),
                {
                    "enabled": True,
                    "skills": {
                        "alpha": {"agents": {"claude-code": True, "codex": False}},
                        "beta": {"agents": None},
                    },
                },
                seeded_links={
                    "claude-code": {"alpha": "bundle/alpha", "beta": "bundle/beta"},
                    "codex": {"alpha": "bundle/alpha", "beta": "bundle/beta"},
                },
            )

            report = self._build(temp_root)

            skill_entries = {entry["name"]: entry for entry in report["sections"]["skills"]}
            self.assertEqual(skill_entries["Claude Code"]["status"], "ok")
            self.assertEqual(skill_entries["Claude Code"]["message"], "2 个受管链接")
            self.assertEqual(skill_entries["Codex"]["status"], "error")
            self.assertIn("存在额外受管 skill 链接", skill_entries["Codex"]["message"])
            self.assertIn("alpha", skill_entries["Codex"]["message"])

    def test_matrix_mode_inbox_notice_lists_unlisted_skills(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root, _config, _skill_cfg = self._write_doctor_fixture(
                temp_dir,
                self._named_skills("alpha", "beta"),
                {"enabled": True, "new_skill_default": "disabled", "skills": {"alpha": {"agents": None}}},
                seeded_links={
                    "claude-code": {"alpha": "bundle/alpha"},
                    "codex": {"alpha": "bundle/alpha"},
                },
            )

            report = self._build(temp_root)

            skill_entries = {entry["name"]: entry for entry in report["sections"]["skills"]}
            self.assertEqual(skill_entries["Claude Code"]["status"], "ok")
            self.assertEqual(skill_entries["Claude Code"]["message"], "1 个受管链接")
            self.assertEqual(skill_entries["Codex"]["status"], "ok")
            inbox_entry = skill_entries.get("demo-repo 收件箱")
            self.assertIsNotNone(inbox_entry, report["sections"]["skills"])
            self.assertEqual(inbox_entry["status"], "notice")
            self.assertIn("1 个新发现 skill 待显式启用", inbox_entry["message"])
            self.assertIn("beta", inbox_entry["message"])
            # 收件箱里的 skill 不构成任何 agent 的期望链接
            for entry in report["sections"]["skills"]:
                self.assertNotIn("缺失", entry["message"])

    def test_legacy_policy_doctor_output_matches_explicit_matrix_policy(self):
        # 升格等价：旧式定义与"范围内全部 skill 对 collection agents 启用"的显式矩阵
        # 必须产出逐字节相同的 skills 段。
        sections = []
        for policy in (
            {"enabled": True},
            {"enabled": True, "skills": {"alpha": {"agents": None}, "beta": {"agents": None}}},
        ):
            with tempfile.TemporaryDirectory() as temp_dir:
                temp_root, _config, _skill_cfg = self._write_doctor_fixture(
                    temp_dir,
                    self._named_skills("alpha", "beta"),
                    policy,
                    seeded_links={
                        "claude-code": {"alpha": "bundle/alpha", "beta": "bundle/beta"},
                        "codex": {"alpha": "bundle/alpha", "beta": "bundle/beta"},
                    },
                )
                report = self._build(temp_root)
                sections.append(report["sections"]["skills"])

        self.assertEqual(sections[0], sections[1])

    def test_matrix_unknown_skill_name_surfaces_resolution_error(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root, _config, _skill_cfg = self._write_doctor_fixture(
                temp_dir,
                self._named_skills("alpha"),
                {"enabled": True, "skills": {"ghost": {"agents": None}}},
            )

            report = self._build(temp_root)

            skill_entries = {entry["name"]: entry for entry in report["sections"]["skills"]}
            for label in ("Claude Code", "Codex"):
                self.assertEqual(skill_entries[label]["status"], "error")
                self.assertIn("技能托管目标解析失败", skill_entries[label]["message"])
                self.assertIn("ghost", skill_entries[label]["message"])

    def test_matrix_mode_honors_disabled_list_over_matrix(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root, _config, _skill_cfg = self._write_doctor_fixture(
                temp_dir,
                self._named_skills("alpha", "beta"),
                {
                    "enabled": True,
                    "disabled": ["alpha"],
                    "skills": {"alpha": {"agents": None}, "beta": {"agents": None}},
                },
                seeded_links={"claude-code": {"alpha": "bundle/alpha", "beta": "bundle/beta"}},
            )

            report = self._build(temp_root)

            skill_entries = {entry["name"]: entry for entry in report["sections"]["skills"]}
            self.assertEqual(skill_entries["Claude Code"]["status"], "error")
            self.assertIn("存在额外受管 skill 链接", skill_entries["Claude Code"]["message"])
            self.assertIn("alpha", skill_entries["Claude Code"]["message"])

    def test_legacy_policy_expectation_ignores_disabled_list(self):
        # 升格等价护栏：旧式定义下 doctor 期望集保持 v1 语义（disabled 名单只由链接侧
        # 收敛），证明矩阵/disabled 感知没有泄漏进旧路径。
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root, _config, _skill_cfg = self._write_doctor_fixture(
                temp_dir,
                self._named_skills("alpha", "beta"),
                {"enabled": True, "disabled": ["alpha"]},
                seeded_links={"claude-code": {"beta": "bundle/beta"}},
            )

            report = self._build(temp_root)

            skill_entries = {entry["name"]: entry for entry in report["sections"]["skills"]}
            self.assertEqual(skill_entries["Claude Code"]["status"], "error")
            self.assertIn("skill 链接缺失", skill_entries["Claude Code"]["message"])
            self.assertIn("alpha", skill_entries["Claude Code"]["message"])

    def test_discovery_inbox_excludes_disabled_listed_skills(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root, config, skill_cfg = self._write_doctor_fixture(
                temp_dir,
                self._named_skills("alpha", "beta", "gamma"),
                {
                    "enabled": True,
                    "new_skill_default": "disabled",
                    "disabled": ["gamma"],
                    "skills": {"alpha": {"agents": None}},
                },
            )

            inbox = collect_skill_discovery_inbox(temp_root, config, skill_cfg)

            # gamma 在 disabled 名单里（已有显式决定），不属于"新发现待启用"
            self.assertEqual(inbox, {"demo-repo": ["beta"]})
