import type { Freshness } from "./contract";

/**
 * 程序：技能与 MCP 依赖的命令行程序（docs/PROGRAM-LIFECYCLE.md）。引擎 `programs:status` 的数据形状，
 * 以及界面上用到的纯函数（管理方式的叫法、MCP 条目里版本号的改写）。智能体本身不在这里。
 */
export type ProgramMode = "managed" | "delegated" | "runner" | "observed" | "missing";

export interface ProgramInstall {
  path: string;
  real: string;
  owner: string;
  program: string | null;
  version: string | null;
  app: string | null;
  package_dir: string | null;
}

export interface ProgramRunner {
  server: string;
  agent: string;
  runner: "npx" | "uvx";
  version: string | null;
  pinned: boolean;
}

export interface ProgramDependent {
  kind: "skill" | "mcp";
  name: string;
  agents: string[];
  source: "declared" | "user" | "mcp";
  bins: string[];
  any: boolean;
}

export interface ProgramProblem {
  code: "missing" | "missing-no-install" | "multiple" | "unpinned" | "runner-missing" | "version-unknown" | "unused" | string;
  bin?: string;
  bins?: string[];
  paths?: string[];
  server?: string;
  agent?: string;
  version?: string | null;
}

export interface ProgramElement {
  id: string;
  program: string;
  kind: string;
  name: string;
  bins: string[];
  all_bins: string[];
  mode: ProgramMode;
  owner: string | null;
  version: string | null;
  installs: ProgramInstall[];
  install: { kind: string; package: string; bins: string[] }[];
  runners: ProgramRunner[];
  dependents: ProgramDependent[];
  problems: ProgramProblem[];
  commands: { install?: string; upgrade?: string; uninstall?: string };
  managed_versions: string[];
  managed_current: string | null;
  freshness: Freshness;
}

export interface ProgramSuggestion {
  skill: string;
  bin: string;
  program: string | null;
  line: string;
  found: boolean;
}

export interface ProgramBackup {
  id: string;
  program: string;
  version: string | null;
  reason: "upgrade" | "adopt" | "before-restore" | string;
  created_at: string;
}

export interface ProgramsData {
  domain: "programs";
  elements: ProgramElement[];
  suggestions: ProgramSuggestion[];
  user_path: { dirs: string[]; entry_candidates: string[]; error: string | null };
  entry_dir: string | null;
  allow_scripts: string[];
  links: Record<string, string>;
  backups: ProgramBackup[];
}

export interface ProgramPlanStep {
  kind: "move" | "exec" | "link" | "unlink" | "remove" | "current" | "meta" | "backup-meta" | "entry-dir";
  from?: string;
  to?: string;
  path?: string;
  target?: string;
  command?: string;
  version?: string;
  dir?: string;
}

export interface ProgramPlan {
  program: string;
  action: string;
  target_version: string | null;
  steps: ProgramPlanStep[];
  notes: string[];
  scripts_ignored: boolean;
  backup: string | null;
  manual: string[];
  scripts_skipped?: string[];
}

export const MODE_LABEL: Record<ProgramMode, string> = {
  managed: "本工具安装",
  delegated: "代管",
  runner: "按需拉取",
  observed: "只观察",
  missing: "未安装",
};

export const MODE_HINT: Record<ProgramMode, string> = {
  managed: "装在本工具目录，按版本各一份，可换版本、退回旧版本",
  delegated: "装在原处（npm 全局 / uv tool），本工具调用原来的包管理器升级、回退",
  runner: "MCP 启动时由 npx / uvx 按条目里写的版本拉取；管理的是条目里的版本号",
  observed: "由 Homebrew、系统或别的应用管理，本工具只显示、给命令",
  missing: "哪里都没有",
};

const OWNER_LABEL: Record<string, string> = {
  managed: "本工具",
  "npm-global": "npm 全局",
  "uv-tool": "uv tool",
  homebrew: "Homebrew",
  app: "应用自带",
  system: "系统",
  unknown: "来源不明",
};

export function ownerLabel(install: ProgramInstall): string {
  if (install.owner === "app" && install.app) return `${install.app} 自带`;
  return OWNER_LABEL[install.owner] ?? install.owner;
}

/** 显示用的名字：按路径认出的程序只显示命令名，路径放在「装在哪」。 */
export function displayName(program: { kind: string; name: string; bins: string[] }): string {
  return program.kind === "path" ? program.bins[0] ?? program.name.split("/").pop() ?? program.name : program.name;
}

export function isExactVersion(version: string): boolean {
  return /^[0-9a-f]{40}$/.test(version) || /^\d[0-9A-Za-z.+-]*$/.test(version);
}

/** 提交号只显示前 7 位，版本号原样。 */
export function shortVersion(version: string | null | undefined): string {
  if (!version) return "";
  return /^[0-9a-f]{40}$/.test(version) ? version.slice(0, 7) : version;
}

/**
 * MCP 条目参数里把版本改成 target：npx 的 `包@版本`、uvx 的 `包==版本` 与 `git+地址@引用`。
 * 找不到可改的位置返回 null（不猜）。其余参数原样保留。
 */
export function pinArgs(runner: "npx" | "uvx", args: string[], target: string): string[] | null {
  const next = [...args];
  if (runner === "npx") {
    const explicit = next.findIndex((arg) => arg === "--package" || arg === "-p");
    const index = explicit >= 0 ? explicit + 1 : next.findIndex((arg) => !arg.startsWith("-"));
    if (index < 0 || index >= next.length) return null;
    const spec = next[index];
    const at = spec.startsWith("@") ? spec.indexOf("@", 1) : spec.indexOf("@");
    const name = at > 0 ? spec.slice(0, at) : spec;
    next[index] = `${name}@${target}`;
    return next;
  }
  const from = next.indexOf("--from");
  const index = from >= 0 ? from + 1 : next.findIndex((arg) => !arg.startsWith("-"));
  if (index < 0 || index >= next.length) return null;
  const spec = next[index];
  if (spec.startsWith("git+")) {
    const at = spec.lastIndexOf("@");
    const base = at > "git+https://".length ? spec.slice(0, at) : spec;
    next[index] = `${base}@${target}`;
    return next;
  }
  const name = spec.split(/==|\[|@/)[0];
  next[index] = `${name}==${target}`;
  return next;
}

/** 这个程序现在跑的版本 → 可升级到的版本（探测结果）。 */
export function upgradeTarget(element: ProgramElement): string | null {
  const fresh = element.freshness;
  if (!fresh?.upstream) return null;
  if (fresh.state === "behind") return fresh.upstream;
  // 按需拉取、没固定版本的：固定到上游当前版本
  if (element.mode === "runner" && element.runners.some((runner) => !runner.pinned)) return fresh.upstream;
  return null;
}
