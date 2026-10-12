import { describe, expect, it } from "vitest";
import App from "../App";
import AgentsPage from "../pages/AgentsPage";
import { setSettingsClient } from "../lib/settings";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

const targets = [
  { id: "claude-code", name: "Claude Code", group: "claude-code", kind: "claude", skills_dir: "~/.claude/skills", mcp_path: "~/.claude.json" },
  { id: "codex", name: "Codex", group: "codex", kind: "codex", skills_dir: "~/.codex/skills", mcp_path: "~/.codex/config.toml" },
  { id: "existing-a", name: "Existing A", group: "codex", kind: "codex", config_dir: "~/.existing-a" },
];

function renderApp() {
  const settings = { appVersion: "0.1.0", contractVersion: "v1", resourceDir: "/tmp/resources", dataDir: "/tmp/data", updaterConfigured: false, preferences: { logRetentionDays: 30 } };
  setSettingsClient({ get: async () => settings, save: async () => { throw new Error("Read-only page must not save"); } });
  return renderWithEngine(<App />, {
    "resources:targets": makeEnvelope({ targets }),
    "skills:status": makeEnvelope({ domain: "skills", elements: [] }),
    "mcp:status": makeEnvelope({ domain: "mcp", elements: [] }),
    "policy:get": makeEnvelope({ layers: { sqlite: null } }),
  });
}

describe("read-only assistant listing", () => {
  it("shows an empty result without creating targets", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, { "resources:targets": makeEnvelope({ targets: [] }) });
    expect(await screen.findByText("暂无助手目标")).toBeInTheDocument();
    expect(screen.queryByRole("button")).toBeNull();
    expect(calls.map((call) => call.command)).toEqual(["resources:targets"]);
  });

  it("shows read errors instead of an empty or successful listing", async () => {
    const error = makeEnvelope({}, { ok: false, errors: [{ code: "READ_ERROR", message: "Cannot read resource metadata", element: null }] });
    const { calls } = renderWithEngine(<AgentsPage />, { "resources:targets": error });
    expect(await screen.findByRole("alert")).toHaveTextContent("Cannot read resource metadata");
    expect(screen.queryByText("暂无助手目标")).toBeNull();
    expect(calls.map((call) => call.command)).toEqual(["resources:targets"]);
  });

  it("opens from navigation and reads only resource metadata", async () => {
    const { calls } = renderApp();
    const nav = await screen.findByRole("navigation");
    await userEvent.click(within(nav).getByRole("button", { name: "助手" }));
    const list = await screen.findByRole("region", { name: "助手列表" });
    expect(within(list).getByRole("heading", { name: "Claude Code" })).toBeInTheDocument();
    expect(within(list).getByText("~/.claude/skills")).toBeInTheDocument();
    expect(within(list).getByText("~/.codex/config.toml")).toBeInTheDocument();
    expect(within(list).getByText("Existing A")).toBeInTheDocument();
    expect(within(list).getByText("~/.existing-a")).toBeInTheDocument();
    expect(within(list).queryByRole("button")).toBeNull();
    expect(within(list).queryByRole("textbox")).toBeNull();
    calls.length = 0;
    await userEvent.click(within(nav).getByRole("button", { name: "技能" }));
    await userEvent.click(within(nav).getByRole("button", { name: "助手" }));
    expect(await screen.findByRole("region", { name: "助手列表" })).toBeInTheDocument();
    expect(calls.every((call) => call.command === "resources:targets")).toBe(true);
  });

  it("can be found as agent in search without account queries or writes", async () => {
    const { calls } = renderApp();
    await screen.findByRole("heading", { name: "技能" });
    calls.length = 0;
    await userEvent.keyboard("{Meta>}k{/Meta}");
    await userEvent.type(screen.getByTestId("command-palette-input"), "agent");
    await userEvent.click(await screen.findByTestId("command-item-page:agents"));
    expect(await screen.findByRole("region", { name: "助手列表" })).toBeInTheDocument();
    expect(calls.every((call) => ["resources:targets", "skills:status", "mcp:status"].includes(call.command))).toBe(true);
    expect(screen.queryByRole("button", { name: /登录|切换账号|创建账号|编辑账号/ })).toBeNull();
  });
});
