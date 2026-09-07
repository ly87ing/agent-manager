"""Byte-for-byte parity: './manage.sh update' (legacy path) vs
'./manage.sh system:converge --json' (engine path used by the GUI).

Two independent sandboxes (identical repo copies, separate HOMEs) run the two
paths offline; every managed artifact — HOME tree (mcp JSON/TOML targets, skill
link topology, rules targets, alias blocks) plus repo-level hook targets — is
diffed recursively after path normalization. The comparator itself is proven
by a negative test that tampers one side and must go red.
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


def _normalized_payload(path, replacements):
    """Bytes of a regular file (or readlink of a symlink) with sandbox paths masked."""
    if path.is_symlink():
        data = os.readlink(path)
    else:
        data = path.read_bytes().decode("utf-8", errors="replace")
    for needle, token in replacements:
        data = data.replace(needle, token)
    return data


def diff_trees(dir_a, dir_b, replacements_a, replacements_b):
    """Recursive structural+content diff of two trees. Returns sorted diffs."""
    diffs = []

    def entries(base):
        result = {}
        for dirpath, dirnames, filenames in os.walk(base):
            dirnames[:] = sorted(d for d in dirnames if d != ".git")
            keep = []
            for name in dirnames:
                full = Path(dirpath) / name
                if full.is_symlink():
                    # 指向目录的软链（skill 链接拓扑）要记录但不递归进入。
                    result[str(full.relative_to(base))] = full
                else:
                    keep.append(name)
            dirnames[:] = keep
            for name in sorted(filenames):
                full = Path(dirpath) / name
                result[str(full.relative_to(base))] = full
        return result

    map_a, map_b = entries(dir_a), entries(dir_b)
    for rel in sorted(set(map_a) | set(map_b)):
        path_a, path_b = map_a.get(rel), map_b.get(rel)
        if path_a is None:
            diffs.append("only-in-B: %s" % rel)
            continue
        if path_b is None:
            diffs.append("only-in-A: %s" % rel)
            continue
        if path_a.is_symlink() != path_b.is_symlink():
            diffs.append("link-type-mismatch: %s" % rel)
            continue
        if _normalized_payload(path_a, replacements_a) != _normalized_payload(path_b, replacements_b):
            diffs.append("content-differs: %s" % rel)
    return sorted(diffs)


class ConvergeParityTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = Path(tempfile.mkdtemp(prefix="agent_manager_parity_test_"))
        self.addCleanup(shutil.rmtree, self.temp_dir, ignore_errors=True)

    # -------------------------------------------------------------- fixture

    def _build_side(self, side):
        side_dir = self.temp_dir / side
        root = side_dir / "repo"
        home = side_dir / "home"
        home.mkdir(parents=True)
        (home / ".claude").mkdir()
        (home / ".codex").mkdir()
        shutil.copytree(ROOT, root, ignore=COPY_IGNORE)
        for candidate in (
            ROOT / "engine-rs" / "target" / "release" / "agent-manager-engine",
            ROOT / "engine-rs" / "target" / "debug" / "agent-manager-engine",
        ):
            if candidate.is_file() and os.access(candidate, os.X_OK):
                dst = root / "engine-rs" / "target" / candidate.parent.name / "agent-manager-engine"
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(candidate, dst)
                break
        for name in ("manager.local.json", "manager.synced.json"):
            (root / "config" / name).unlink(missing_ok=True)

        def write_config(name, obj):
            (root / "config" / name).write_text(json.dumps(obj, indent=2) + "\n", encoding="utf-8")

        # 集合仓库：两个 skill，git 形态（link_skills 只接管 git 缓存）。
        coll = root / "agent-skill" / "repositories" / "coll"
        for skill in ("skill-a", "skill-b"):
            skill_dir = coll / skill
            skill_dir.mkdir(parents=True)
            (skill_dir / "SKILL.md").write_text(
                "---\nname: %s\ndescription: parity\n---\n" % skill, encoding="utf-8"
            )
        subprocess.run(["git", "init", "-q", str(coll)], check=True, capture_output=True)

        write_config("agents.json", {})
        write_config("tools.json", {"tools": {}})
        write_config(
            "manager.json",
            {
                "agents": {"claude-code": {"enabled": True}, "codex": {"enabled": True}},
                "tools": {},
                "skills": {"coll": {"enabled": True}},
                "mcp": {"alpha": {"enabled": True}, "beta": {"enabled": True}},
                "browser_us_sync": {"enabled": False},
            },
        )
        write_config(
            "mcp.json",
            {
                "servers": {
                    "alpha": {"command": "alpha-cmd", "args": ["--x"], "env": {}, "agents": ["agent-a"]},
                    "beta": {"command": "beta-cmd", "args": ["--y"], "env": {}, "agents": ["agent-b"]},
                },
                "agent_paths": {
                    "agent-a": str(home / ".claude.json"),
                    "agent-b": {"path": str(home / ".codex" / "config.toml"), "format": "toml"},
                },
            },
        )
        write_config(
            "skills.json",
            {
                "repositories": {"coll": {"url": "https://example.invalid/coll.git", "agents": ["claude-code", "codex"]}},
                "agents": {
                    "claude-code": str(home / ".claude" / "skills"),
                    "codex": str(home / ".codex" / "skills"),
                },
            },
        )
        (root / "config" / "rules-targets.list").write_text(
            "claude_global|global|rules/dist/claude/CLAUDE.md|{{HOME}}/.claude/CLAUDE.md\n"
            "codex_global|global|rules/dist/codex/AGENTS.md|{{HOME}}/.codex/AGENTS.md\n",
            encoding="utf-8",
        )
        write_config(
            "claude-hooks.json",
            {"settings_path": "../.claude/settings.json", "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "echo parity"}]}]}},
        )
        write_config(
            "codex-hooks.json",
            {"hooks_path": "../.codex/hooks.json", "hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "echo parity-codex"}]}]}},
        )
        write_config(
            "shell-aliases.json",
            {"timezone": "UTC", "lang": "C", "aliases": {"ll": "ls -l", "gs": "git status"}},
        )
        return root, home

    def _env_for(self, side):
        side_dir = self.temp_dir / side
        env = os.environ.copy()
        env["HOME"] = str(side_dir / "home")
        env["AGENT_MANAGER_LOCK_FILE"] = str(side_dir / "lock")
        env["AGENT_MANAGER_LOG_PREFIX"] = str(side_dir / "logs" / "sync")
        env["AGENT_MANAGER_SNAPSHOT_DIR"] = str(side_dir / "snapshots")
        env["AGENT_MANAGER_UPSTREAM_CACHE"] = str(side_dir / "upstream.json")
        return env

    def _run_manage(self, root, side, *argv):
        (self.temp_dir / side / "logs").mkdir(exist_ok=True)
        result = subprocess.run(
            [str(root / "manage.sh"), *argv],
            cwd=root,
            env=self._env_for(side),
            capture_output=True,
            text=True,
            check=False,
            timeout=300,
        )
        self.assertEqual(result.returncode, 0, "%s: %s" % (argv, result.stdout[-2000:] + result.stderr[-2000:]))
        return result

    def _replacements(self, side):
        side_dir = self.temp_dir / side
        return [(str(side_dir / "repo"), "<ROOT>"), (str(side_dir / "home"), "<HOME>")]

    def _diff_sides(self, root_a, home_a, root_b, home_b):
        rep_a, rep_b = self._replacements("A"), self._replacements("B")
        diffs = diff_trees(home_a, home_b, rep_a, rep_b)
        for rel_target in (".claude/settings.json", ".codex/hooks.json"):
            path_a, path_b = root_a / rel_target, root_b / rel_target
            self.assertTrue(path_a.exists() and path_b.exists(), rel_target)
            if _normalized_payload(path_a, rep_a) != _normalized_payload(path_b, rep_b):
                diffs.append("content-differs: repo/%s" % rel_target)
        return diffs

    # -------------------------------------------------------------- tests

    def test_converge_produces_byte_identical_artifacts_to_update(self):
        root_a, home_a = self._build_side("A")
        root_b, home_b = self._build_side("B")

        self._run_manage(root_a, "A", "--offline", "update")
        result_b = self._run_manage(root_b, "B", "--offline", "system:converge", "--json")
        payload = json.loads(result_b.stdout)
        self.assertTrue(payload["ok"], result_b.stdout)

        diffs = self._diff_sides(root_a, home_a, root_b, home_b)
        self.assertEqual(diffs, [], "update 与 converge 产物不一致:\n%s" % "\n".join(diffs))

        # 覆盖清单（防静默漏面）：受管产物必须真实存在才被比较。
        for expected in (
            ".claude.json",
            ".codex/config.toml",
            ".claude/skills/skill-a",
            ".codex/skills/skill-b",
            ".claude/CLAUDE.md",
            ".codex/AGENTS.md",
            ".zshrc",
            ".zprofile",
        ):
            self.assertTrue((home_a / expected).exists() or (home_a / expected).is_symlink(), expected)

    def test_comparator_goes_red_when_sides_diverge(self):
        # 反例：对账函数必须能发现人为差异（否则逐字节一致断言没有牙齿）。
        root_a, home_a = self._build_side("A")
        root_b, home_b = self._build_side("B")
        self._run_manage(root_a, "A", "--offline", "update")
        self._run_manage(root_b, "B", "--offline", "system:converge", "--json")

        target = home_b / ".claude.json"
        target.write_text(target.read_text(encoding="utf-8") + "\n", encoding="utf-8")
        (home_b / ".claude" / "skills" / "skill-b").unlink()
        diffs = self._diff_sides(root_a, home_a, root_b, home_b)
        self.assertIn("content-differs: .claude.json", diffs)
        self.assertIn("only-in-A: .claude/skills/skill-b", diffs)


if __name__ == "__main__":
    unittest.main()
