import contextlib
import http.client
import importlib.util
import io
import subprocess
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

    def test_perform_healthcheck_retries_transient_connection_failures(self):
        attempts = []
        sleep_calls = []

        def fake_request_ok(method, url, *, bearer_token=None, timeout=10):
            attempts.append(("ok", method, url, bearer_token, timeout))
            if len(attempts) < 3:
                raise RuntimeError("Remote end closed connection without response")

        def fake_request_json(method, url, *, payload=None, bearer_token=None, timeout=10):
            attempts.append(("json", method, url, bearer_token, timeout))
            return {"ok": True}

        cc_bridge_manage.perform_healthcheck(
            {"server_port": "5674", "admin_password": "secret"},
            attempts=3,
            delay_seconds=0.25,
            sleep_func=lambda seconds: sleep_calls.append(seconds),
            request_ok_func=fake_request_ok,
            request_json_func=fake_request_json,
        )

        self.assertEqual(
            attempts,
            [
                ("ok", "GET", "http://127.0.0.1:5674/login", None, 10),
                ("ok", "GET", "http://127.0.0.1:5674/login", None, 10),
                ("ok", "GET", "http://127.0.0.1:5674/login", None, 10),
                ("json", "GET", "http://127.0.0.1:5674/admin/dashboard", "secret", 10),
            ],
        )
        self.assertEqual(sleep_calls, [0.25, 0.25])

    def test_perform_healthcheck_retries_remote_disconnect_during_startup(self):
        attempts = []
        sleep_calls = []

        def fake_request_ok(method, url, *, bearer_token=None, timeout=10):
            attempts.append(("ok", method, url, bearer_token, timeout))
            if len(attempts) < 3:
                raise http.client.RemoteDisconnected("Remote end closed connection without response")

        def fake_request_json(method, url, *, payload=None, bearer_token=None, timeout=10):
            attempts.append(("json", method, url, bearer_token, timeout))
            return {"ok": True}

        cc_bridge_manage.perform_healthcheck(
            {"server_port": "5674", "admin_password": "secret"},
            attempts=3,
            delay_seconds=0.25,
            sleep_func=lambda seconds: sleep_calls.append(seconds),
            request_ok_func=fake_request_ok,
            request_json_func=fake_request_json,
        )

        self.assertEqual(
            attempts,
            [
                ("ok", "GET", "http://127.0.0.1:5674/login", None, 10),
                ("ok", "GET", "http://127.0.0.1:5674/login", None, 10),
                ("ok", "GET", "http://127.0.0.1:5674/login", None, 10),
                ("json", "GET", "http://127.0.0.1:5674/admin/dashboard", "secret", 10),
            ],
        )
        self.assertEqual(sleep_calls, [0.25, 0.25])

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

    def test_install_runtime_installs_latest_release_when_runtime_is_missing(self):
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

            calls = []

            def fake_run_command(argv, cwd=None):
                calls.append((tuple(argv), None if cwd is None else str(cwd)))
                return 0

            result = cc_bridge_manage.install_runtime(
                root_dir=root,
                fetch_tag_func=lambda: "v1.8.3",
                run_command_func=fake_run_command,
                healthcheck_func=lambda settings: None,
                now_func=lambda: "20260422-090000",
                sync_secrets_func=lambda root_dir: None,
            )

            self.assertEqual(result["current_tag"], "v1.8.3")
            self.assertEqual(result["latest_tag"], "v1.8.3")
            self.assertEqual(result["runtime_dir"], str(runtime_dir))
            self.assertTrue((runtime_dir / "compose.yaml").exists())
            self.assertEqual(
                (runtime_dir / "state" / "current-tag").read_text(encoding="utf-8"),
                "v1.8.3\n",
            )
            self.assertEqual(
                calls,
                [
                    (("docker", "compose", "-f", "compose.yaml", "down"), str(runtime_dir)),
                    (("docker", "compose", "-f", "compose.yaml", "pull"), str(runtime_dir)),
                    (("docker", "compose", "-f", "compose.yaml", "up", "-d"), str(runtime_dir)),
                ],
            )

    def test_start_runtime_updates_before_start_when_new_release_exists(self):
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
            (runtime_dir / "state").mkdir(parents=True)
            (runtime_dir / "state" / "current-tag").write_text("v1.8.2\n", encoding="utf-8")
            (runtime_dir / "state" / "previous-tag").write_text("v1.8.1\n", encoding="utf-8")
            (runtime_dir / "compose.yaml").write_text("services:\n", encoding="utf-8")

            calls = []

            def fake_run_command(argv, cwd=None):
                calls.append((tuple(argv), None if cwd is None else str(cwd)))
                return 0

            result = cc_bridge_manage.start_runtime(
                root_dir=root,
                fetch_tag_func=lambda: "v1.8.3",
                run_command_func=fake_run_command,
                healthcheck_func=lambda settings: None,
                now_func=lambda: "20260422-090500",
                sync_secrets_func=lambda root_dir: None,
            )

            self.assertEqual(result["current_tag"], "v1.8.3")
            self.assertEqual(result["previous_tag"], "v1.8.2")
            self.assertEqual(result["latest_tag"], "v1.8.3")
            self.assertEqual(
                calls,
                [
                    (("docker", "compose", "-f", "compose.yaml", "down"), str(runtime_dir)),
                    (("docker", "compose", "-f", "compose.yaml", "pull"), str(runtime_dir)),
                    (("docker", "compose", "-f", "compose.yaml", "up", "-d"), str(runtime_dir)),
                ],
            )

    def test_start_runtime_uses_compose_up_when_already_on_latest_release(self):
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
            runtime_dir.mkdir(parents=True)
            (runtime_dir / "compose.yaml").write_text("services:\n", encoding="utf-8")
            (runtime_dir / "state").mkdir(parents=True)
            (runtime_dir / "state" / "current-tag").write_text("v1.8.3\n", encoding="utf-8")
            (runtime_dir / "state" / "previous-tag").write_text("v1.8.2\n", encoding="utf-8")

            calls = []

            def fake_run_command(argv, cwd=None):
                calls.append((tuple(argv), None if cwd is None else str(cwd)))
                return 0

            result = cc_bridge_manage.start_runtime(
                root_dir=root,
                fetch_tag_func=lambda: "v1.8.3",
                run_command_func=fake_run_command,
                healthcheck_func=lambda settings: None,
                sync_secrets_func=lambda root_dir: None,
            )

            self.assertEqual(result["current_tag"], "v1.8.3")
            self.assertEqual(result["latest_tag"], "v1.8.3")
            self.assertEqual(
                (runtime_dir / "state" / "latest-known-tag").read_text(encoding="utf-8"),
                "v1.8.3\n",
            )
            self.assertEqual(
                calls,
                [
                    (("docker", "compose", "-f", "compose.yaml", "up", "-d"), str(runtime_dir)),
                ],
            )

    def test_stop_runtime_uses_compose_stop(self):
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
            runtime_dir.mkdir(parents=True)
            (runtime_dir / "compose.yaml").write_text("services:\n", encoding="utf-8")

            calls = []

            def fake_run_command(argv, cwd=None):
                calls.append((tuple(argv), None if cwd is None else str(cwd)))
                return 0

            result = cc_bridge_manage.stop_runtime(
                root_dir=root,
                run_command_func=fake_run_command,
                sync_secrets_func=lambda root_dir: None,
            )

            self.assertEqual(result["runtime_dir"], str(runtime_dir))
            self.assertTrue(result["stopped"])
            self.assertEqual(
                calls,
                [
                    (("docker", "compose", "-f", "compose.yaml", "stop"), str(runtime_dir)),
                ],
            )

    def test_status_report_includes_runtime_metadata_and_update_available(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            runtime_dir = Path(temp_dir) / "runtime"
            (runtime_dir / "state").mkdir(parents=True)
            (runtime_dir / "state" / "current-tag").write_text("v1.8.2\n", encoding="utf-8")
            (runtime_dir / "state" / "previous-tag").write_text("v1.8.1\n", encoding="utf-8")

            report = cc_bridge_manage.build_status_report(
                runtime_dir=runtime_dir,
                latest_tag="v1.8.3",
                docker_ps_output="NAME STATUS\ncc-bridge Up 3 minutes\n",
                server_port="5674",
                container_status="running",
                pid=4321,
            )

            self.assertEqual(
                report,
                {
                    "current_tag": "v1.8.2",
                    "previous_tag": "v1.8.1",
                    "latest_tag": "v1.8.3",
                    "update_available": True,
                    "runtime_dir": str(runtime_dir),
                    "container_name": "cc-bridge",
                    "container_status": "running",
                    "pid": 4321,
                    "port": "5674",
                    "base_url": "http://127.0.0.1:5674",
                    "login_url": "http://127.0.0.1:5674/login",
                    "admin_url": "http://127.0.0.1:5674/admin/dashboard",
                    "claude_code": {
                        "settings_path": "~/.claude/settings.json",
                        "base_url_env": "ANTHROPIC_BASE_URL",
                        "auth_token_env": "ANTHROPIC_AUTH_TOKEN",
                        "auth_token_placeholder": "<cc_bridge_api_token>",
                        "token_note": "Use a cc-bridge API token for Claude Code; the admin password only works for admin APIs.",
                        "shell_exports": {
                            "ANTHROPIC_BASE_URL": "http://127.0.0.1:5674",
                            "ANTHROPIC_AUTH_TOKEN": "<cc_bridge_api_token>",
                        },
                        "settings_json": {
                            "env": {
                                "ANTHROPIC_BASE_URL": "http://127.0.0.1:5674",
                                "ANTHROPIC_AUTH_TOKEN": "<cc_bridge_api_token>",
                            }
                        },
                    },
                    "docker_status": "NAME STATUS\ncc-bridge Up 3 minutes",
                },
            )

    def test_collect_status_includes_container_pid_and_urls(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            runtime_dir = root / "runtime"
            secrets_dir = root / "secrets"
            secrets_dir.mkdir()
            (secrets_dir / "cc-bridge.env").write_text(
                (
                    f"CC_BRIDGE_RUNTIME_DIR={runtime_dir}\n"
                    "SERVER_PORT=6789\n"
                    "DATABASE_DRIVER=sqlite\n"
                    "ADMIN_PASSWORD=secret\n"
                ),
                encoding="utf-8",
            )
            runtime_dir.mkdir(parents=True)
            (runtime_dir / "compose.yaml").write_text("services:\n", encoding="utf-8")
            (runtime_dir / "state").mkdir(parents=True)
            (runtime_dir / "state" / "current-tag").write_text("v1.8.2\n", encoding="utf-8")
            (runtime_dir / "state" / "previous-tag").write_text("v1.8.1\n", encoding="utf-8")

            def fake_run_command(argv, cwd=None):
                if tuple(argv) == ("docker", "compose", "-f", "compose.yaml", "ps"):
                    return subprocess.CompletedProcess(
                        argv,
                        0,
                        stdout="NAME STATUS\ncc-bridge Up 2 minutes\n",
                        stderr="",
                    )
                if tuple(argv) == ("docker", "inspect", "cc-bridge"):
                    return subprocess.CompletedProcess(
                        argv,
                        0,
                        stdout='[{"State": {"Status": "running", "Pid": 4321}}]',
                        stderr="",
                    )
                raise AssertionError(f"unexpected command: {argv!r}")

            report = cc_bridge_manage.collect_status(
                root,
                run_command_func=fake_run_command,
                fetch_latest_tag_func=lambda runtime_dir: "v1.8.3",
            )

            self.assertEqual(report["pid"], 4321)
            self.assertEqual(report["container_status"], "running")
            self.assertEqual(report["port"], "6789")
            self.assertEqual(report["base_url"], "http://127.0.0.1:6789")
            self.assertEqual(report["login_url"], "http://127.0.0.1:6789/login")
            self.assertEqual(report["admin_url"], "http://127.0.0.1:6789/admin/dashboard")
            self.assertEqual(
                report["claude_code"]["shell_exports"]["ANTHROPIC_BASE_URL"],
                "http://127.0.0.1:6789",
            )

    def test_print_status_includes_claude_code_configuration_snippet(self):
        report = {
            "current_tag": "v1.8.3",
            "previous_tag": "",
            "latest_tag": "v1.8.3",
            "update_available": False,
            "runtime_dir": "/tmp/runtime",
            "container_name": "cc-bridge",
            "container_status": "running",
            "pid": 4321,
            "port": "5674",
            "base_url": "http://127.0.0.1:5674",
            "login_url": "http://127.0.0.1:5674/login",
            "admin_url": "http://127.0.0.1:5674/admin/dashboard",
            "claude_code": {
                "settings_path": "~/.claude/settings.json",
                "base_url_env": "ANTHROPIC_BASE_URL",
                "auth_token_env": "ANTHROPIC_AUTH_TOKEN",
                "auth_token_placeholder": "<cc_bridge_api_token>",
                "token_note": "Use a cc-bridge API token for Claude Code; the admin password only works for admin APIs.",
                "shell_exports": {
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:5674",
                    "ANTHROPIC_AUTH_TOKEN": "<cc_bridge_api_token>",
                },
                "settings_json": {
                    "env": {
                        "ANTHROPIC_BASE_URL": "http://127.0.0.1:5674",
                        "ANTHROPIC_AUTH_TOKEN": "<cc_bridge_api_token>",
                    }
                },
            },
            "docker_status": "NAME STATUS\ncc-bridge Up 3 minutes",
        }

        buffer = io.StringIO()
        with contextlib.redirect_stdout(buffer):
            cc_bridge_manage.print_status(report, as_json=False)

        rendered = buffer.getvalue()
        self.assertIn("claude_code_settings_path: ~/.claude/settings.json", rendered)
        self.assertIn("export ANTHROPIC_BASE_URL=http://127.0.0.1:5674", rendered)
        self.assertIn("export ANTHROPIC_AUTH_TOKEN=<cc_bridge_api_token>", rendered)
        self.assertIn('"ANTHROPIC_BASE_URL": "http://127.0.0.1:5674"', rendered)
