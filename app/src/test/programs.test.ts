import { describe, expect, it } from "vitest";
import { buildInbox } from "../lib/inbox";
import { pinArgs, upgradeTarget, type ProgramElement } from "../lib/programs";

function program(overrides: Partial<ProgramElement>): ProgramElement {
  return {
    id: "programs:npm:x",
    program: "npm:x",
    kind: "npm",
    name: "x",
    bins: ["x"],
    all_bins: ["x"],
    mode: "delegated",
    owner: "npm-global",
    version: "1.0.0",
    installs: [],
    install: [],
    runners: [],
    dependents: [{ kind: "skill", name: "s", agents: ["claude-code"], source: "declared", bins: ["x"], any: false }],
    problems: [],
    commands: {},
    managed_versions: [],
    managed_current: null,
    freshness: { state: "unknown", current: "1.0.0", upstream: null, checked_at: null, probe: null, cache_age_sec: null },
    ...overrides,
  };
}

describe("MCP 条目里的版本号改写：只改版本那一段", () => {
  it("npx：包@版本（含作用域包、--package 写法），其余参数原样", () => {
    expect(pinArgs("npx", ["-y", "chrome-devtools-mcp@1.9.0", "--isolated"], "1.10.1")).toEqual(["-y", "chrome-devtools-mcp@1.10.1", "--isolated"]);
    expect(pinArgs("npx", ["-y", "@scope/srv"], "2.0.0")).toEqual(["-y", "@scope/srv@2.0.0"]);
    expect(pinArgs("npx", ["-y", "@scope/srv@latest", "--port", "1"], "2.0.0")).toEqual(["-y", "@scope/srv@2.0.0", "--port", "1"]);
    expect(pinArgs("npx", ["--package", "pkg@1", "run"], "3.0.0")).toEqual(["--package", "pkg@3.0.0", "run"]);
  });

  it("uvx：git 源的 @引用改成提交号；PyPI 包写成 ==版本；认不出位置返回 null", () => {
    const commit = "7a2968335f2198b966864de1ce3655c8e485a653";
    expect(pinArgs("uvx", ["--from", "git+https://github.com/oraios/serena@main", "serena", "start"], commit)).toEqual([
      "--from", `git+https://github.com/oraios/serena@${commit}`, "serena", "start",
    ]);
    expect(pinArgs("uvx", ["--from", "git+https://github.com/o/r", "r"], commit)).toEqual(["--from", `git+https://github.com/o/r@${commit}`, "r"]);
    expect(pinArgs("uvx", ["mcp-server-fetch==0.6.2"], "0.7.0")).toEqual(["mcp-server-fetch==0.7.0"]);
    expect(pinArgs("uvx", ["mcp-server-fetch"], "0.7.0")).toEqual(["mcp-server-fetch==0.7.0"]);
    expect(pinArgs("npx", ["-y"], "1.0.0")).toBeNull();
  });

  it("可升级到的版本：落后时给新版本；按需拉取没固定时给上游当前版本；其余没有", () => {
    expect(upgradeTarget(program({ freshness: { ...program({}).freshness, state: "behind", upstream: "1.1.0" } }))).toBe("1.1.0");
    expect(upgradeTarget(program({ freshness: { ...program({}).freshness, state: "current", upstream: "1.0.0" } }))).toBeNull();
    const runner = program({ mode: "runner", runners: [{ server: "serena", agent: "codex", runner: "uvx", version: "main", pinned: false }], freshness: { ...program({}).freshness, state: "unknown", upstream: "abc" } });
    expect(upgradeTarget(runner)).toBe("abc");
  });
});

describe("待处理里的程序：只汇总，处理去程序页", () => {
  const input = (programs: ProgramElement[]) => ({ skills: [], mcp: [], mcpTargets: [], rules: [], agents: [], profiles: [], aliases: [], programs });

  it("缺失、没固定版本、有新版本、同名多份、运行器缺失、可清理各自归到统一状态", () => {
    const items = buildInbox(input([
      program({ program: "npm:a", name: "a", mode: "missing", problems: [{ code: "missing", bins: ["a"] }] }),
      program({ program: "bin:b", name: "b", mode: "missing", problems: [{ code: "missing-no-install", bins: ["b"] }] }),
      program({ program: "git:s", name: "s", mode: "runner", problems: [{ code: "unpinned", server: "serena", agent: "codex", version: "main" }, { code: "unpinned", server: "serena", agent: "claude-code", version: "main" }] }),
      program({ program: "npm:c", name: "c", freshness: { ...program({}).freshness, state: "behind", upstream: "2.0.0" } }),
      program({ program: "npm:d", name: "d", problems: [{ code: "multiple", bin: "d", paths: ["~/.local/bin/d", "/usr/local/bin/d"] }] }),
      program({ program: "npm:e", name: "e", mode: "runner", problems: [{ code: "runner-missing", bin: "npx" }] }),
      program({ program: "npm:f", name: "f", mode: "managed", dependents: [], problems: [{ code: "unused" }] }),
    ]));
    const byId = Object.fromEntries(items.map((item) => [item.id, item]));
    expect(byId["program:npm:a:missing"]).toMatchObject({ page: "programs", state: "missing", fix: { kind: "open", label: "去安装", target: "programs:npm:a" } });
    expect(byId["program:bin:b:missing"].fix.label).toBe("补充安装方式");
    expect(byId["program:git:s:unpinned"]).toMatchObject({ state: "update", detail: expect.stringContaining("serena") });
    expect(items.filter((item) => item.id.startsWith("program:git:s")).length).toBe(1);
    expect(byId["program:npm:c:behind"]).toMatchObject({ state: "update", detail: "1.0.0 → 2.0.0" });
    expect(byId["program:npm:d:multiple:d"].state).toBe("drifted");
    expect(byId["program:npm:e:runner"].state).toBe("broken");
    expect(byId["program:npm:f:unused"].state).toBe("cleanup");
  });
});
