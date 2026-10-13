import { beforeAll, describe, expect, it, vi } from "vitest";
import { act } from "@testing-library/react";
import { toast } from "sonner";
import App from "../App";
import SettingsPage from "../pages/SettingsPage";
import { FileDiff } from "../components/FileDiff";
import { getLanguage, setLanguage, t } from "../lib/i18n";
import { messages } from "../lib/messages";
import { setSettingsClient, settingsSchema } from "../lib/settings";
import type { SettingsData } from "../lib/contract";
import { makeEnvelope } from "./fixtures";
import { chooseOption, renderWithEngine, screen, userEvent, within } from "./render-with-engine";

const defaults = settingsSchema.parse({
  preferences: { logRetentionDays: 30 }, appVersion: "test", contractVersion: "v1",
  resourceDir: "/resources", dataDir: "/data", updaterConfigured: false,
});
const routes = {
  "skills:status": makeEnvelope({ domain: "skills", elements: [] }, { command: "skills.status" }),
  "policy:get": makeEnvelope({ layers: { sqlite: null } }, { command: "policy.get" }),
};

describe("application language", () => {
  beforeAll(async () => { await Promise.all([import("../pages/LaunchPage"), import("../pages/SkillsPage")]); });

  it("defaults older preferences to English and rejects unsupported languages", () => {
    expect(defaults.preferences.language).toBe("en");
    expect(settingsSchema.safeParse({ ...defaults, preferences: { ...defaults.preferences, language: "fr" } }).success).toBe(false);
  });

  it("opens in English without writing preferences, independently of browser language", async () => {
    const save = vi.fn(async () => defaults);
    vi.spyOn(navigator, "language", "get").mockReturnValue("zh-CN");
    setSettingsClient({ get: async () => defaults, save });
    const { calls } = renderWithEngine(<App />, routes);
    expect(await screen.findByRole("heading", { name: "Launch configurations" })).toBeInTheDocument();
    expect(document.documentElement.lang).toBe("en");
    const nav = screen.getByRole("navigation");
    expect(within(nav).getByRole("button", { name: "Settings" })).toBeInTheDocument();
    expect(save).not.toHaveBeenCalled();
    expect(calls.map(({ command }) => command).filter((command) =>
      !["launch:catalog", "launch:list", "skills:status", "programs:status", "policy:get", "resources:targets"].includes(command))).toEqual([]);
  });

  it("applies only saved language, switches navigation, and retains it on reopen", async () => {
    let saved: SettingsData = structuredClone(defaults);
    const save = vi.fn(async (preferences) => (saved = { ...saved, preferences }));
    setSettingsClient({ get: async () => saved, save });
    const success = vi.spyOn(toast, "success");
    const first = renderWithEngine(<App />, routes);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    await chooseOption(await screen.findByRole("combobox", { name: "Language" }), "zh-CN");
    expect(getLanguage()).toBe("en");
    expect(save).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Save settings" }));
    expect(await screen.findByRole("heading", { name: "设置" })).toBeInTheDocument();
    expect(document.documentElement.lang).toBe("zh-CN");
    expect(success).toHaveBeenLastCalledWith("设置已保存");
    expect(save).toHaveBeenCalledExactlyOnceWith({ language: "zh-CN", logRetentionDays: 30 });
    expect(within(screen.getByRole("navigation")).getByRole("button", { name: "技能" })).toBeInTheDocument();
    first.unmount();

    renderWithEngine(<SettingsPage />, {});
    const language = await screen.findByRole("combobox", { name: "语言" });
    expect(language).toHaveAttribute("data-value", "zh-CN");
    expect(save).toHaveBeenCalledTimes(1);
    await chooseOption(language, "en");
    await userEvent.click(screen.getByRole("button", { name: "保存设置" }));
    expect(await screen.findByRole("heading", { name: "Settings" })).toBeInTheDocument();
    expect(success).toHaveBeenLastCalledWith("Settings saved");
    expect(saved.preferences.language).toBe("en");
  });

  it("keeps the actual language on save failure and leaves the draft available to retry", async () => {
    const save = vi.fn(async () => { throw new Error("disk full"); });
    setSettingsClient({ get: async () => defaults, save });
    renderWithEngine(<SettingsPage />, {});
    await chooseOption(await screen.findByRole("combobox", { name: "Language" }), "zh-CN");
    await userEvent.click(screen.getByRole("button", { name: "Save settings" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Unable to save settings: disk full");
    expect(getLanguage()).toBe("en");
    expect(screen.getByRole("combobox", { name: "Language" })).toHaveAttribute("data-value", "zh-CN");
    expect(screen.getByRole("button", { name: "Save settings" })).toBeEnabled();
  });

  it("does not let a canceled old settings read replace a newly saved language", async () => {
    let saved: SettingsData = structuredClone(defaults);
    let resolveOld!: (data: SettingsData) => void;
    const oldRead = new Promise<SettingsData>((resolve) => { resolveOld = resolve; });
    let reads = 0;
    setSettingsClient({
      get: async () => ++reads === 2 ? oldRead : saved,
      save: async (preferences) => (saved = { ...saved, preferences }),
    });
    const view = renderWithEngine(<SettingsPage />, {});
    await screen.findByRole("combobox", { name: "Language" });
    const refetch = view.queryClient.refetchQueries({ queryKey: ["settings"] });
    await chooseOption(screen.getByRole("combobox", { name: "Language" }), "zh-CN");
    await userEvent.click(screen.getByRole("button", { name: "Save settings" }));
    expect(await screen.findByRole("heading", { name: "设置" })).toBeInTheDocument();
    await act(async () => { resolveOld(defaults); await refetch; });
    expect(getLanguage()).toBe("zh-CN");
    expect(screen.getByRole("heading", { name: "设置" })).toBeInTheDocument();
  });

  it("preserves user content and interpolated text even when it matches a translation key", () => {
    setLanguage("en");
    expect(t("编辑 {0}", "设置 {1} <test>")).toBe("Edit 设置 {1} <test>");
    renderWithEngine(<FileDiff original={null} content="设置" path="/YOUR_RULE.md" />, {});
    expect(screen.getByText("设置")).toBeInTheDocument();
  });

  it("keeps every translation complete and preserves interpolation arguments", () => {
    for (const [key, english] of Object.entries(messages)) {
      expect(english, key).not.toMatch(/UNUSED|[\u3400-\u9fff]/);
      expect([...(english.match(/\{\d+\}/g) ?? [])].sort(), key)
        .toEqual([...(key.match(/\{\d+\}/g) ?? [])].sort());
    }
  });
});
