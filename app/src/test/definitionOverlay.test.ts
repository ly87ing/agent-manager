import { describe, expect, it } from "vitest";
import { definitionKeyPrefix, definitionOrigin, buildDefinitionRemoval } from "../lib/definitionOverlay";
import type { PolicyLayers } from "../lib/contract";

const layers: PolicyLayers = {
  sqlite: { mcp: { definitions: { "my-proxy": { command: "npx" } } } },
};

describe("定义覆盖层移除决策", () => {
  it("键前缀：mcp.definitions / skills.repositories", () => {
    expect(definitionKeyPrefix("mcp")).toBe("mcp.definitions");
    expect(definitionKeyPrefix("skills")).toBe("skills.repositories");
  });

  it("定义在 local → 写入移除标记，防止下层定义恢复", () => {
    const spec = buildDefinitionRemoval(layers, "mcp", "my-proxy");
    expect(spec).toEqual({ kind: "tombstone", layer: "local", key: "mcp.definitions.my-proxy" });
  });

  it("定义在 sqlite → 写入 local tombstone", () => {
    const spec = buildDefinitionRemoval(layers, "mcp", "my-proxy");
    expect(spec).toEqual({ kind: "tombstone", layer: "local", key: "mcp.definitions.my-proxy" });
  });

  it("sqlite 没有定义（内置条目）→ 写 local tombstone", () => {
    const spec = buildDefinitionRemoval(layers, "mcp", "chrome-devtools");
    expect(spec).toEqual({ kind: "tombstone", layer: "local", key: "mcp.definitions.chrome-devtools" });
  });

  it("skills 段同理（repositories 键）", () => {
    const none: PolicyLayers = { sqlite: null };
    expect(buildDefinitionRemoval(none, "skills", "agent-skills")).toEqual({
      kind: "tombstone",
      layer: "local",
      key: "skills.repositories.agent-skills",
    });
  });

  it("definitionOrigin：builtin / sqlite 两态", () => {
    expect(definitionOrigin(layers, "mcp", "chrome-devtools")).toBe("builtin");
    expect(definitionOrigin(layers, "mcp", "my-proxy")).toBe("sqlite");
  });
});
