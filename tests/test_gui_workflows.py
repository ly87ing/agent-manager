import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class GuiWorkflowContractTests(unittest.TestCase):
    def test_release_workflow_uses_explicit_tag_and_app_project(self):
        workflow = (ROOT / ".github/workflows/gui-release.yml").read_text()
        self.assertIn('description: "Existing v* tag to build and upload"', workflow)
        self.assertIn("format('refs/tags/{0}', inputs.tag)", workflow)
        self.assertIn("projectPath: app", workflow)
        self.assertIn("tagName: ${{ env.RELEASE_TAG }}", workflow)
        self.assertIn("node-version: 24", workflow)
        self.assertIn('RELEASE_TAG" =~ ^v[0-9]+\\.[0-9]+\\.[0-9]+$', workflow)
        self.assertIn('git rev-parse "refs/tags/$RELEASE_TAG^{commit}"', workflow)
        self.assertIn('pkg_ver="$(node -p \'require("./app/package.json").version\')"', workflow)
        self.assertIn('tauri_ver="$(node -p \'require("./app/src-tauri/tauri.conf.json").version\')"', workflow)
        self.assertIn('cargo_ver="$(python3 -c', workflow)
        self.assertIn("import tomllib", workflow)
        self.assertIn("import tomli as tomllib", workflow)
        self.assertIn("Enable updater artifacts when signing key is present", workflow)
        self.assertIn("createUpdaterArtifacts=true", workflow)
        self.assertNotIn("os: windows-latest", workflow)

    def test_windows_ci_does_not_claim_to_build_an_installer(self):
        workflow = (ROOT / ".github/workflows/gui-ci.yml").read_text()
        windows = workflow.split("app-windows:", 1)[1]
        self.assertIn("continue-on-error: true", windows)
        self.assertIn("仅保留静态检查与单测", windows)
        self.assertNotIn("tauri build --bundles nsis", windows)

    def test_release_validation_script_execution_in_git_fixtures(self):
        import os
        import subprocess
        import tempfile

        workflow_text = (ROOT / ".github/workflows/gui-release.yml").read_text(encoding="utf-8")

        # 使用标准库提取指定步骤的 run: | 脚本区块，不依赖外部 PyYAML
        lines = workflow_text.splitlines()
        in_step = False
        in_run = False
        run_lines = []
        run_indent = None
        for line in lines:
            if "name: Validate release tag, checkout, and version consistency" in line:
                in_step = True
                continue
            if in_step and not in_run:
                stripped = line.lstrip()
                if stripped.startswith("run: |"):
                    in_run = True
                    run_indent = len(line) - len(stripped) + 2
                    continue
            elif in_run:
                if not line.strip():
                    run_lines.append("")
                    continue
                indent = len(line) - len(line.lstrip())
                if indent >= run_indent:
                    run_lines.append(line[run_indent:])
                else:
                    break
        self.assertTrue(run_lines, "Could not extract validation script block from workflow")
        script = "\n".join(run_lines) + "\n"

        def run_validation(
            tag: str,
            pkg_ver: str = "1.0.0",
            tauri_ver: str = "1.0.0",
            cargo_ver: str = "1.0.0",
            extra_commit: bool = False,
        ) -> tuple[int, str]:
            with tempfile.TemporaryDirectory() as tmpdir:
                tmp_path = Path(tmpdir)
                repo = tmp_path / "repo"
                repo.mkdir()
                fake_home = tmp_path / "home"
                fake_home.mkdir()

                env = {
                    "HOME": str(fake_home),
                    "PATH": os.environ.get("PATH", ""),
                    "RELEASE_TAG": tag,
                    "GIT_CONFIG_NOSYSTEM": "1",
                    "GIT_CONFIG_GLOBAL": str(fake_home / ".gitconfig"),
                    "GIT_AUTHOR_NAME": "YOUR_TEST_AUTHOR",
                    "GIT_AUTHOR_EMAIL": "YOUR_TEST_AUTHOR@example.com",
                    "GIT_COMMITTER_NAME": "YOUR_TEST_COMMITTER",
                    "GIT_COMMITTER_EMAIL": "YOUR_TEST_COMMITTER@example.com",
                    "GIT_TERMINAL_PROMPT": "0",
                }

                # 临时 git 仓库禁用签名、禁用 hooks、隔离账号配置
                subprocess.run(["git", "init"], cwd=repo, env=env, check=True, capture_output=True)
                subprocess.run(["git", "config", "commit.gpgsign", "false"], cwd=repo, env=env, check=True)
                subprocess.run(["git", "config", "tag.gpgsign", "false"], cwd=repo, env=env, check=True)
                subprocess.run(["git", "config", "core.hooksPath", "/dev/null"], cwd=repo, env=env, check=True)
                subprocess.run(["git", "config", "user.name", "YOUR_TEST_USER"], cwd=repo, env=env, check=True)
                subprocess.run(["git", "config", "user.email", "YOUR_TEST_EMAIL@example.com"], cwd=repo, env=env, check=True)

                (repo / "app/src-tauri").mkdir(parents=True)
                (repo / "app/package.json").write_text(f'{{"version": "{pkg_ver}"}}', encoding="utf-8")
                (repo / "app/src-tauri/tauri.conf.json").write_text(f'{{"version": "{tauri_ver}"}}', encoding="utf-8")
                (repo / "app/src-tauri/Cargo.toml").write_text(f'[package]\nname="app"\nversion="{cargo_ver}"\n', encoding="utf-8")

                subprocess.run(["git", "add", "."], cwd=repo, env=env, check=True)
                subprocess.run(["git", "commit", "-m", "initial"], cwd=repo, env=env, check=True, capture_output=True)
                if tag:
                    subprocess.run(["git", "tag", tag], cwd=repo, env=env, check=True)
                if extra_commit:
                    (repo / "extra.txt").write_text("extra", encoding="utf-8")
                    subprocess.run(["git", "add", "."], cwd=repo, env=env, check=True)
                    subprocess.run(["git", "commit", "-m", "extra commit"], cwd=repo, env=env, check=True, capture_output=True)

                # 按 GitHub Actions 真实 bash 默认选项 -eo pipefail 执行
                res = subprocess.run(["bash", "-eo", "pipefail", "-c", script], cwd=repo, env=env, capture_output=True, text=True)
                return res.returncode, res.stderr

        # 1. 正常 tag 与三处版本完全一致 -> exit code 0
        code, stderr = run_validation("v1.0.0")
        self.assertEqual(code, 0, f"Expected 0 for valid tag, got: {stderr}")

        # 2. 非 vX.Y.Z 格式 tag -> exit code 1
        code, stderr = run_validation("1.0.0")
        self.assertEqual(code, 1)
        self.assertIn("release tag must match vX.Y.Z", stderr)

        code, stderr = run_validation("v1.0")
        self.assertEqual(code, 1)
        self.assertIn("release tag must match vX.Y.Z", stderr)

        # 3. Tag 指向提交与当前 HEAD 不一致 -> exit code 1
        code, stderr = run_validation("v1.0.0", extra_commit=True)
        self.assertEqual(code, 1)
        self.assertIn("checked out commit does not match release tag v1.0.0", stderr)

        # 4. 版本不一致（三个版本错配全部反例） -> exit code 1
        code, stderr = run_validation("v1.0.0", pkg_ver="1.0.1")
        self.assertEqual(code, 1)
        self.assertIn("Version mismatch", stderr)

        code, stderr = run_validation("v1.0.0", tauri_ver="1.0.1")
        self.assertEqual(code, 1)
        self.assertIn("Version mismatch", stderr)

        code, stderr = run_validation("v1.0.0", cargo_ver="1.0.1")
        self.assertEqual(code, 1)
        self.assertIn("Version mismatch", stderr)


if __name__ == "__main__":
    unittest.main()
