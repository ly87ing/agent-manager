import { describe, expect, it } from "vitest";
import { definitionKeyPrefix, buildDefinitionRemoval } from "../lib/definitionOverlay";
import type { PolicyLayers } from "../lib/contract";

// MCP 不再走定义覆盖层（条目以各助手配置文件为准，由 mcp:apply 逐条编辑）；
// 原先以 mcp.definitions 举例的用例改用 agents.definitions 验证同一套覆盖层语义。
const layers: PolicyLayers = {
  sqlite: { agents: { definitions: { "my-agent": { bin: "my-agent" } } } },
};

describe("定义覆盖层移除决策", () => {
  it("键前缀：agents.definitions / skills.repositories", () => {
    expect(definitionKeyPrefix("agents")).toBe("agents.definitions");
    expect(definitionKeyPrefix("skills")).toBe("skills.repositories");
  });

  it("定义在 sqlite → 写入 local tombstone，防止下层定义恢复", () => {
    const spec = buildDefinitionRemoval(layers, "agents", "my-agent");
    expect(spec).toEqual({ kind: "tombstone", layer: "local", key: "agents.definitions.my-agent" });
  });

  it("sqlite 没有定义（内置条目）→ 写 local tombstone", () => {
    const spec = buildDefinitionRemoval(layers, "agents", "codex");
    expect(spec).toEqual({ kind: "tombstone", layer: "local", key: "agents.definitions.codex" });
  });

  it("skills 段同理（repositories 键）", () => {
    const none: PolicyLayers = { sqlite: null };
    expect(buildDefinitionRemoval(none, "skills", "agent-skills")).toEqual({
      kind: "tombstone",
      layer: "local",
      key: "skills.repositories.agent-skills",
    });
  });
});
