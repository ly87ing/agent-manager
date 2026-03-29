import tempfile
import unittest
from pathlib import Path

from src.secret_scan import scan_repo_for_secrets, scan_text_for_secrets


class SecretScanTests(unittest.TestCase):
    def test_scan_text_detects_live_like_credentials(self):
        openai_key = "sk-proj-" + "abcdefghijklmnopqrstuvwxyz12"
        augment_token = "ace_" + "abcdefghijklmnop"
        findings = scan_text_for_secrets(
            '\n'.join(
                [
                    f'OPENAI_API_KEY="{openai_key}"',
                    f'AUGMENT_SESSION_AUTH=\'{{"accessToken":"{augment_token}"}}\'',
                    "",
                ]
            )
        )

        self.assertEqual(
            {(finding.line, finding.kind) for finding in findings},
            {
                (1, "openai_like_key"),
                (2, "augment_session_token"),
            },
        )

    def test_scan_text_ignores_documentation_placeholders(self):
        findings = scan_text_for_secrets(
            '\n'.join(
                [
                    'OPENAI_API_KEY="sk-abc123..."',
                    'AUGMENT_SESSION_AUTH=\'{"accessToken":"ace_...","tenantURL":"...","scopes":[]}\'',
                    'AUGMENT_SESSION_AUTH=\'{"accessToken":"new_token"}\'',
                    "",
                ]
            )
        )

        self.assertEqual(findings, [])

    def test_repo_scan_skips_secrets_directory_but_flags_repo_file(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root_dir = Path(temp_dir) / "repo"
            root_dir.mkdir()
            openai_key = "sk-proj-" + "abcdefghijklmnopqrstuvwxyz12"
            (root_dir / "README.md").write_text("# repo\n", encoding="utf-8")
            (root_dir / "leak.txt").write_text(
                f'OPENAI_API_KEY="{openai_key}"\n',
                encoding="utf-8",
            )
            (root_dir / "secrets").mkdir()
            (root_dir / "secrets" / "mcp.env").write_text(
                f'OPENAI_API_KEY="{openai_key}"\n',
                encoding="utf-8",
            )

            findings = scan_repo_for_secrets(root_dir)

            self.assertEqual([(finding.path, finding.kind) for finding in findings], [("leak.txt", "openai_like_key")])
