import unittest
from pathlib import Path

from src.command_runner import prepare_command


class CommandRunnerTests(unittest.TestCase):
    def test_prepare_command_splits_simple_string_without_shell(self):
        prepared, use_shell = prepare_command("git -C ~/.oh-my-zsh pull")

        self.assertFalse(use_shell)
        self.assertEqual(prepared[0], "git")
        self.assertEqual(prepared[1], "-C")
        self.assertEqual(prepared[2], str(Path.home() / ".oh-my-zsh"))
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
