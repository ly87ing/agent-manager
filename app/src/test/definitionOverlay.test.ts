import { describe, expect, it } from "vitest";
import { definitionKeyPrefix, definitionOrigin, planDefinitionRemoval } from "../lib/definitionOverlay";
import type { PolicyLayers } from "../lib/contract";

const layers: PolicyLayers = {
  baseline: { mcp: { "chrome-devtools": { enabled: true } } },
  synced: { mcp: { definitions: { "synced-srv": { command: "npx" } } } },
  local: { mcp: { definitions: { "my-proxy": { command: "npx" } } } },
};

describe("定义覆盖层移除决策", () => {
  it("键前缀：mcp.definitions / skills.repositories", () => {
    expect(definitionKeyPrefix("mcp")).toBe("mcp.definitions");
    expect(definitionKeyPrefix("skills")).toBe("skills.repositories");
  });

  it("定义在 local → 写入移除标记，防止下层定义恢复", () => {
    const plan = planDefinitionRemoval(layers, "mcp", "my-proxy", "local");
    expect(plan).toEqual({ kind: "tombstone", layer: "local", key: "mcp.definitions.my-proxy" });
  });

  it("定义在 synced → 在默认 local 层遮蔽", () => {
    const plan = planDefinitionRemoval(layers, "mcp", "synced-srv", "local");
    expect(plan).toEqual({ kind: "tombstone", layer: "local", key: "mcp.definitions.synced-srv" });
  });

  it("两层都没有（基线条目）→ 往默认层写 tombstone", () => {
    const plan = planDefinitionRemoval(layers, "mcp", "chrome-devtools", "local");
    expect(plan).toEqual({ kind: "tombstone", layer: "local", key: "mcp.definitions.chrome-devtools" });
    const syncedDefault = planDefinitionRemoval(layers, "mcp", "chrome-devtools", "synced");
    expect(syncedDefault.layer).toBe("synced");
  });

  it("skills 段同理（repositories 键）", () => {
    const none: PolicyLayers = { baseline: {}, synced: null, local: null };
    expect(planDefinitionRemoval(none, "skills", "agent-skills", "local")).toEqual({
      kind: "tombstone",
      layer: "local",
      key: "skills.repositories.agent-skills",
    });
  });

  it("definitionOrigin：baseline / local / synced 三态", () => {
    expect(definitionOrigin(layers, "mcp", "chrome-devtools")).toBe("baseline");
    expect(definitionOrigin(layers, "mcp", "my-proxy")).toBe("local");
    expect(definitionOrigin(layers, "mcp", "synced-srv")).toBe("synced");
  });
});
