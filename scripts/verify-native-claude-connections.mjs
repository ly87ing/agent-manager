// Actual CLI acceptance using owned files. Never clears personal Claude or Codex data.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

const binary = path.resolve(process.argv[2] ?? "engine-rs/target/debug/agent-manager-engine");
const work = fs.mkdtempSync(path.join(os.tmpdir(), "am-claude-connections-"));
const home = path.join(work, "home"), data = path.join(work, "manager"), root = path.join(home, ".claude");
const env = { PATH: process.env.PATH, TMPDIR: os.tmpdir(), HOME: home, USERPROFILE: home, AGENT_MANAGER_DATA_DIR: data };
const protectedFiles = new Map();
function write(file, value) { fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, value); }
function protect(file, value) { write(file, value); protectedFiles.set(file, fs.readFileSync(file)); }
function retained() { for (const [file, value] of protectedFiles) assert.deepEqual(fs.readFileSync(file), value, `Protected file changed: ${path.basename(file)}`); }
function run(verb, args = []) {
  const result = spawnSync(binary, [`claude-data:${verb}`, "--json", ...args], { env, cwd: work, encoding: "utf8", timeout: 30_000, maxBuffer: 1024 * 1024 });
  assert.ifError(result.error);
  const envelope = JSON.parse(result.stdout);
  assert.equal(result.status, envelope.ok ? 0 : 1);
  assert(!result.stdout.includes("PRIVATE_"), "A secret value escaped into output");
  return envelope;
}
const select = item => ["--connection-items", JSON.stringify([item])];
function preview(item, extra = []) {
  const result = run("preview", [...select(item), ...extra]);
  assert(result.ok, JSON.stringify(result.errors));
  assert(result.data.can_clear, JSON.stringify(result.data.blockers));
  return result.data;
}
function clear(item, before, extra = []) {
  const result = run("clear", [...select(item), ...extra, "--base", before.base, "--confirm"]);
  assert(result.ok, JSON.stringify(result.errors));
  assert.equal(result.data.status, "complete");
  assert(result.data.removed && result.data.retained_verified);
  retained();
}
let sessionKey;
function launch(args) {
  const result = spawnSync("/bin/launchctl", args, { encoding: "utf8", timeout: 5000 });
  assert.ifError(result.error); assert.equal(result.status, 0, "Synthetic launch environment operation failed");
  return result.stdout;
}
try {
  const file = path.join(root, "settings.json");
  const original = '{ "env": {"ANTHROPIC_API_KEY":"PRIVATE_SELECTED", "ANTHROPIC_CUSTOM_HEADERS":"PRIVATE_OTHER", "KEEP":"PRIVATE_KEEP"}, "apiKeyHelper":"PRIVATE_HELPER", "hooks": { "before": [1,2] }, "model":"YOUR_MODEL" }\n';
  write(file, original);
  protect(path.join(root, "history.jsonl"), "PRIVATE_SESSION");
  protect(path.join(root, ".credentials.json"), "PRIVATE_CREDENTIALS");
  protect(path.join(home, ".codex/auth.json"), "PRIVATE_CODEX_AUTH");
  protect(path.join(home, ".codex/config.toml"), "PRIVATE_CODEX_CONFIG");
  protect(path.join(home, "project/main.rs"), "YOUR_SOURCE");
  const startup = path.join(home, ".zshrc");
  write(startup, "# YOUR_COMMENT\r\nexport PATH=/YOUR_PATH\r\nexport ANTHROPIC_BASE_URL='PRIVATE_URL' # keep\r\nexport HTTP_PROXY='PRIVATE_SHARED_PROXY'\r\nexport CODEX_HOME=/YOUR_CODEX\r\n");
  const initial = run("inspect-connections");
  assert(initial.ok); assert(!fs.existsSync(data)); retained();
  assert(initial.data.sources.find(source => source.id === ".zshrc").items.some(item => item.key === "HTTP_PROXY" && !item.editable));
  if (process.platform !== "darwin") {
    assert.equal(initial.data.clear_supported, false);
    console.log(JSON.stringify({ status: "inspection_verified", writes: "not_supported", work }));
    process.exit(0);
  }
  const selected = "settings:ANTHROPIC_API_KEY";
  const before = preview(selected);
  assert(!fs.existsSync(data), "Preview wrote Manager data");
  assert.equal(fs.readFileSync(file, "utf8"), original, "Preview changed settings");
  for (const args of [["--connection-items"], ["--connection-items", "[]"], ["--connection-items", '["codex:ANTHROPIC_API_KEY"]'], ["--connection-items", "null"], ["--connection-root", root], [...select(selected), "--files-only"]]) {
    assert(!run("clear", [...args, "--base", before.base, "--confirm"]).ok);
    assert.equal(fs.readFileSync(file, "utf8"), original); retained();
  }
  assert(!fs.existsSync(data), "Rejected requests created an audit/database");
  write(file, original + " ");
  assert(!run("clear", [...select(selected), "--base", before.base, "--confirm"]).ok);
  assert.equal(fs.readFileSync(file, "utf8"), original + " ");
  clear(selected, preview(selected));
  const expected = original.replace('{"ANTHROPIC_API_KEY":"PRIVATE_SELECTED", "ANTHROPIC_CUSTOM_HEADERS":"PRIVATE_OTHER", "KEEP":"PRIVATE_KEEP"}', '{"ANTHROPIC_CUSTOM_HEADERS":"PRIVATE_OTHER","KEEP":"PRIVATE_KEEP"}') + " ";
  assert.equal(fs.readFileSync(file, "utf8"), expected);
  const remaining = JSON.parse(expected); delete remaining.apiKeyHelper;
  clear("settings:root:apiKeyHelper", preview("settings:root:apiKeyHelper"));
  assert.deepEqual(JSON.parse(fs.readFileSync(file)), remaining);
  const shell = "shell:.zshrc:3:ANTHROPIC_BASE_URL";
  const shellBefore = fs.readFileSync(startup, "utf8");
  clear(shell, preview(shell));
  assert.equal(fs.readFileSync(startup, "utf8"), shellBefore.replace("export ANTHROPIC_BASE_URL='PRIVATE_URL'", "#"));
  protect(startup, fs.readFileSync(startup));
  const custom = path.join(work, "custom-claude"), customFile = path.join(custom, "settings.json");
  write(customFile, '{"env":{"ANTHROPIC_CUSTOM_HEADERS":"PRIVATE_CUSTOM","KEEP":"PRIVATE_OTHER"},"mcpServers":{}}');
  const customArgs = ["--connection-root", custom], customItem = "settings:ANTHROPIC_CUSTOM_HEADERS";
  clear(customItem, preview(customItem, customArgs), customArgs);
  assert.deepEqual(JSON.parse(fs.readFileSync(customFile)), { env: { KEEP: "PRIVATE_OTHER" }, mcpServers: {} });
  for (const rejected of [path.join(home, ".codex"), home, data, "relative-path"]) {
    assert(!run("inspect-connections", ["--connection-root", rejected]).ok);
  }
  const codexOverride = path.join(work, "custom-codex");
  write(path.join(codexOverride, "settings.json"), original);
  env.CODEX_HOME = codexOverride;
  assert(!run("inspect-connections", ["--connection-root", codexOverride]).ok);
  delete env.CODEX_HOME;
  const codexAlias = path.join(work, "codex-alias-target");
  fs.renameSync(path.join(home,".codex"),codexAlias);
  fs.symlinkSync(codexAlias,path.join(home,".codex"),"dir");
  assert(!run("inspect-connections",["--connection-root",codexAlias]).ok);
  write(path.join(work, "project/package.json"), "{}");
  write(path.join(work, "project/nested/settings.json"), original);
  assert(!run("inspect-connections", ["--connection-root", path.join(work, "project/nested")]).ok);
  write(path.join(work,"worktrees/nested/settings.json"),original);
  assert(!run("inspect-connections",["--connection-root",path.join(work,"worktrees/nested")]).ok);
  write(path.join(custom, ".git"), "gitdir: YOUR_WORKTREE");
  assert(!run("inspect-connections", customArgs).ok);
  fs.unlinkSync(path.join(custom, ".git"));
  fs.symlinkSync(custom, path.join(work, "linked"), "dir");
  assert(!run("inspect-connections", ["--connection-root", path.join(work, "linked")]).ok);
  write(customFile, '{"env":{"ANTHROPIC_API_KEY":"PRIVATE_A","ANTHROPIC_API_KEY":"PRIVATE_B"},"hooks":{}}');
  const damaged = fs.readFileSync(customFile);
  assert(!run("preview", [...select(selected), ...customArgs]).ok);
  assert.deepEqual(fs.readFileSync(customFile), damaged);
  retained();
  if (process.env.AM_TEST_CLAUDE_SESSION === "1") {
    sessionKey = `ANTHROPIC_AM_TEST_${crypto.randomUUID().replaceAll("-", "").toUpperCase()}`;
    assert.equal(launch(["getenv", sessionKey]).trim(), "");
    launch(["setenv", sessionKey, "PRIVATE_SYNTHETIC"]);
    // A lexical reference makes this explicitly named custom variable discoverable.
    write(path.join(home, ".profile"), `# ${sessionKey}\n`);
    const inspected = run("inspect-connections");
    assert(inspected.data.sources.find(source => source.id === "user-environment").items.some(item => item.key === sessionKey && item.editable));
    const sessionItem = `user-env:${sessionKey}`;
    const sessionPreview = preview(sessionItem);
    assert(sessionPreview.shared);
    clear(sessionItem, sessionPreview);
    assert.equal(launch(["getenv", sessionKey]).trim(), "");
    sessionKey = undefined;
  }
  console.log(JSON.stringify({ status: "complete", cases: ["readonly", "cancel", "invalid_selection", "stale", "settings", "root_field", "startup", "custom_root", "protected_paths", "ambiguous_json", "unselected_and_codex_preserved"], session: process.env.AM_TEST_CLAUDE_SESSION === "1" ? "verified" : "not_run", work }));
} finally {
  if (sessionKey) launch(["unsetenv", sessionKey]);
  retained();
  // Keep owned acceptance files until the consumer has reviewed the receipt.
}
