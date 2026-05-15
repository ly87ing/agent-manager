import unittest
from pathlib import Path

from src.command_runner import prepare_command


class CommandRunnerTests(unittest.TestCase):
    def test_prepare_command_splits_simple_string_without_shell(self):
        prepared, use_shell = prepare_command("git -C ~/.agent-manager/tools/repo-helper pull")

        self.assertFalse(use_shell)
        self.assertEqual(prepared[0], "git")
        self.assertEqual(prepared[1], "-C")
        self.assertEqual(prepared[2], str(Path.home() / ".agent-manager" / "tools" / "repo-helper"))
        self.assertEqual(prepared[3], "pull")

    def test_prepare_command_preserves_compound_shell_syntax(self):
        command = "npm install && npx playwright install"

        prepared, use_shell = prepare_command(command)

        self.assertTrue(use_shell)
        self.assertEqual(prepared, command)

    def test_prepare_command_preserves_command_substitution(self):
        command = 'sh -c "$(curl -fsSL https://example.com/install.sh)" "" --unattended'

        prepared, use_shell = prepare_command(command)

        self.assertTrue(use_shell)
        self.assertEqual(prepared, command)

    def test_prepare_command_ignores_shell_markers_inside_quotes(self):
        command = 'python3 -c \'print("a&&b")\''

        prepared, use_shell = prepare_command(command)

        self.assertFalse(use_shell)
        self.assertEqual(prepared, ["python3", "-c", 'print("a&&b")'])

    def test_prepare_command_preserves_env_assignments_for_shell(self):
        command = "FOO=bar git status"

        prepared, use_shell = prepare_command(command)

        self.assertTrue(use_shell)
        self.assertEqual(prepared, command)

    def test_prepare_command_preserves_glob_expansion_for_shell(self):
        command = "echo *.json"

        prepared, use_shell = prepare_command(command)

        self.assertTrue(use_shell)
        self.assertEqual(prepared, command)

    def test_prepare_command_preserves_variable_expansion_for_shell(self):
        command = "echo $HOME"

        prepared, use_shell = prepare_command(command)

        self.assertTrue(use_shell)
        self.assertEqual(prepared, command)
