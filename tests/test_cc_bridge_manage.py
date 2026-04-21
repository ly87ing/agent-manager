import importlib.util
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "scripts" / "cc-bridge" / "manage.py"
SPEC = importlib.util.spec_from_file_location("cc_bridge_manage", MODULE_PATH)
cc_bridge_manage = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(cc_bridge_manage)


class CcBridgeManageTests(unittest.TestCase):
    def test_render_compose_uses_bind_mount_and_explicit_tag(self):
        runtime_dir = Path("/Users/tester/services/cc-bridge")
        rendered = cc_bridge_manage.render_compose_yaml(
            runtime_dir=runtime_dir,
            image_ref="ghcr.io/mamoworks/cc-bridge:v1.8.3",
            server_port="5674",
        )

        self.assertIn('image: "ghcr.io/mamoworks/cc-bridge:v1.8.3"', rendered)
        self.assertIn(f'- "{runtime_dir}/.env.rendered"', rendered)
        self.assertIn(f'- "{runtime_dir}/data:/app/data"', rendered)
        self.assertNotIn("cc-bridge-data:", rendered)
        self.assertNotIn("depends_on:", rendered)

    def test_fetch_latest_release_tag_rejects_missing_tag_name(self):
        with self.assertRaisesRegex(ValueError, "tag_name"):
            cc_bridge_manage.parse_latest_release_tag({"name": "broken"})

    def test_load_runtime_config_requires_sqlite_driver(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            secrets_dir = root / "secrets"
            secrets_dir.mkdir()
            (secrets_dir / "cc-bridge.env").write_text(
                (
                    "CC_BRIDGE_RUNTIME_DIR=/tmp/runtime\n"
                    "DATABASE_DRIVER=postgres\n"
                    "ADMIN_PASSWORD=secret\n"
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "DATABASE_DRIVER"):
                cc_bridge_manage.load_runtime_config(root)

    def test_update_to_latest_release_renders_runtime_and_records_state(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            runtime_dir = root / "runtime"
            secrets_dir = root / "secrets"
            secrets_dir.mkdir()
            (secrets_dir / "cc-bridge.env").write_text(
                (
                    f"CC_BRIDGE_RUNTIME_DIR={runtime_dir}\n"
                    "SERVER_PORT=5674\n"
                    "DATABASE_DRIVER=sqlite\n"
                    "ADMIN_PASSWORD=secret\n"
                ),
                encoding="utf-8",
            )
            (runtime_dir / "data").mkdir(parents=True)
            (runtime_dir / "data" / "claude-code-gateway.db").write_text("sqlite\n", encoding="utf-8")

            calls = []

            def fake_run_command(argv, cwd=None):
                calls.append((tuple(argv), None if cwd is None else str(cwd)))
                return 0

            cc_bridge_manage.update_to_latest_release(
                root_dir=root,
                fetch_tag_func=lambda: "v1.8.3",
                run_command_func=fake_run_command,
                healthcheck_func=lambda settings: None,
                now_func=lambda: "20260421-120000",
                sync_secrets_func=lambda root_dir: None,
            )

            compose_path = runtime_dir / "compose.yaml"
            env_rendered_path = runtime_dir / ".env.rendered"
            state_dir = runtime_dir / "state"
            backup_path = runtime_dir / "backups" / "cc-bridge-data-20260421-120000.tar.gz"

            self.assertTrue(compose_path.exists())
            self.assertTrue(env_rendered_path.exists())
            self.assertTrue(backup_path.exists())
            self.assertEqual((state_dir / "current-tag").read_text(encoding="utf-8"), "v1.8.3\n")
            self.assertEqual((state_dir / "previous-tag").read_text(encoding="utf-8"), "\n")
            self.assertEqual((state_dir / "latest-known-tag").read_text(encoding="utf-8"), "v1.8.3\n")
            self.assertIn("ADMIN_PASSWORD=\"secret\"", env_rendered_path.read_text(encoding="utf-8"))
            self.assertEqual(
                calls,
                [
                    (("docker", "compose", "-f", "compose.yaml", "down"), str(runtime_dir)),
                    (("docker", "compose", "-f", "compose.yaml", "pull"), str(runtime_dir)),
                    (("docker", "compose", "-f", "compose.yaml", "up", "-d"), str(runtime_dir)),
                ],
            )

    def test_status_report_marks_update_available_when_latest_differs(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            runtime_dir = Path(temp_dir) / "runtime"
            (runtime_dir / "state").mkdir(parents=True)
            (runtime_dir / "state" / "current-tag").write_text("v1.8.2\n", encoding="utf-8")
            (runtime_dir / "state" / "previous-tag").write_text("v1.8.1\n", encoding="utf-8")

            report = cc_bridge_manage.build_status_report(
                runtime_dir=runtime_dir,
                latest_tag="v1.8.3",
                docker_ps_output="NAME STATUS\ncc-bridge Up 3 minutes\n",
            )

            self.assertEqual(
                report,
                {
                    "current_tag": "v1.8.2",
                    "previous_tag": "v1.8.1",
                    "latest_tag": "v1.8.3",
                    "update_available": True,
                    "docker_status": "NAME STATUS\ncc-bridge Up 3 minutes",
                },
            )
