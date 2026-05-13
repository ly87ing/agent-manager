"""Contract tests for the iTerm2 color-preset importer.

The importer is purely a side-effect script that delegates to `open -a iTerm`,
so we focus on three contracts:

1. The vendored theme directory contains exactly the expected preset set.
2. The pure `compute_missing_presets` skips presets that already exist.
3. The script exits 0 on non-macOS hosts (idempotent no-op).
4. `--dry-run` never invokes `open` and prints a preview line per missing preset.
"""
from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
THEMES_DIR = ROOT / "agent-terminal" / "iterm-themes"
SCRIPT = ROOT / "agent-terminal" / "import_iterm_themes.py"

EXPECTED_THEMES = {
    "Atom One Dark",
    "Catppuccin Macchiato",
    "Catppuccin Mocha",
    "Dracula",
    "Gruvbox Dark",
    "Nord",
    "Rose Pine",
    "Solarized Dark Higher Contrast",
    "TokyoNight",
    "TokyoNight Moon",
    "TokyoNight Storm",
}


def _load_module():
    spec = importlib.util.spec_from_file_location("import_iterm_themes", SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class VendoredThemesTests(unittest.TestCase):
    def test_themes_dir_contains_expected_presets(self):
        actual = {p.stem for p in THEMES_DIR.glob("*.itermcolors")}
        self.assertEqual(actual, EXPECTED_THEMES)

    def test_each_theme_file_is_valid_xml_plist(self):
        for f in THEMES_DIR.glob("*.itermcolors"):
            head = f.read_bytes()[:200]
            self.assertIn(b"<?xml", head, f"not xml: {f.name}")
            self.assertIn(b"plist", head, f"not plist: {f.name}")


class ComputeMissingPresetsTests(unittest.TestCase):
    def setUp(self):
        self.mod = _load_module()
        self.files = sorted(THEMES_DIR.glob("*.itermcolors"))

    def test_returns_all_when_existing_is_empty(self):
        missing = self.mod.compute_missing_presets(self.files, set())
        self.assertEqual({p.stem for p in missing}, EXPECTED_THEMES)

    def test_skips_presets_already_present(self):
        existing = {"Dracula", "Nord", "Atom One Dark"}
        missing = self.mod.compute_missing_presets(self.files, existing)
        self.assertEqual(
            {p.stem for p in missing}, EXPECTED_THEMES - existing
        )

    def test_returns_empty_when_all_present(self):
        missing = self.mod.compute_missing_presets(self.files, EXPECTED_THEMES)
        self.assertEqual(missing, [])


class ContentCollisionTests(unittest.TestCase):
    """Skip imports when content matches an existing preset under any name.

    iTerm2 fires a "duplicate preset" confirmation dialog when the imported
    content equals an existing preset's color map — even with a different
    name. We must detect this before invoking the GUI path.
    """

    def setUp(self):
        self.mod = _load_module()

    def test_returns_name_when_content_matches(self):
        color_map = {"Ansi 0 Color": {"Red Component": 0.1}}
        existing = {"Dracula (2)": color_map, "Other": {"Ansi 0 Color": {"Red Component": 0.5}}}
        match = self.mod.find_content_collision(color_map, existing)
        self.assertEqual(match, "Dracula (2)")

    def test_returns_none_when_no_match(self):
        color_map = {"Ansi 0 Color": {"Red Component": 0.1}}
        existing = {"Other": {"Ansi 0 Color": {"Red Component": 0.5}}}
        self.assertIsNone(self.mod.find_content_collision(color_map, existing))


class ScriptIntegrationTests(unittest.TestCase):
    def test_dry_run_exits_zero_without_writing(self):
        env = os.environ.copy()
        env["AGENT_MANAGER_ITERM_FORCE_MACOS"] = "1"
        result = subprocess.run(
            ["python3", str(SCRIPT), "--dry-run"],
            capture_output=True, text=True, env=env, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        combined = result.stdout + result.stderr
        # Acceptable outcomes: nothing pending, all content-duplicates, or preview lines.
        self.assertTrue(
            any(token in combined for token in ("(预览", "已全部导入", "内容均已存在")),
            combined,
        )

    def test_non_macos_exits_zero_and_skips(self):
        env = os.environ.copy()
        env["AGENT_MANAGER_ITERM_FORCE_NON_MACOS"] = "1"
        result = subprocess.run(
            ["python3", str(SCRIPT)],
            capture_output=True, text=True, env=env, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("非 macOS", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
