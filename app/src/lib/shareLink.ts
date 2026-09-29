/**
 * 分享链接：把一个 MCP 服务或技能集合的定义装进 `agentmanager://import?data=…`，
 * 别人粘贴或点开后，用现有的「新增」表单预填、自己检查后保存——链接本身从不直接写入。
 *
 * 只收白名单字段；取值只允许变量引用（`${VAR}`），任何明文取值（可能是凭据）一律拒收；
 * 技能集合只收 git 地址，不收本机路径（换台机器就不成立）。
 */

export const SHARE_PREFIX = "agentmanager://import?data=";

export type SharePayload =
  | {
      kind: "mcp";
      name: string;
      /** stdio：command/args/env；远程：url/headers。取值只能是 ${VAR}。 */
      spec: { command?: string; args?: string[]; env?: Record<string, string>; url?: string; headers?: Record<string, string> };
    }
  | {
      kind: "skills";
      name: string;
      definition: { url: string; skill_paths?: string[] | null; rules_path?: string | null };
    };

const NAME_RE = /^[A-Za-z0-9_-]{1,64}$/;
const VAR_RE = /^\$\{[A-Za-z_][A-Za-z0-9_]*\}$/;

function toBase64Url(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function fromBase64Url(text: string): string {
  const padded = text.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (text.length % 4)) % 4);
  const binary = atob(padded);
  return new TextDecoder().decode(Uint8Array.from(binary, (char) => char.charCodeAt(0)));
}

/** 生成分享链接。调用方保证传入的已是脱敏后的取值；这里再按同样的规则校验一遍。 */
export function encodeShareLink(payload: SharePayload): string {
  const checked = validate(payload);
  if (typeof checked === "string") throw new Error(checked);
  return SHARE_PREFIX + toBase64Url(JSON.stringify({ v: 1, ...checked }));
}

/** 解析粘贴或点开的链接：合法返回内容，否则返回说明原因的文字。 */
export function decodeShareLink(text: string): SharePayload | string {
  const trimmed = text.trim();
  if (!trimmed.startsWith(SHARE_PREFIX)) return "这不是 Agent Manager 的分享链接（应以 agentmanager://import 开头）";
  let raw: unknown;
  try {
    raw = JSON.parse(fromBase64Url(trimmed.slice(SHARE_PREFIX.length)));
  } catch {
    return "链接内容读不出来，可能复制不完整";
  }
  if (!raw || typeof raw !== "object" || (raw as { v?: unknown }).v !== 1) return "不认识的链接版本";
  return validate(raw as SharePayload);
}

function varsOnly(map: unknown, what: string): Record<string, string> | string {
  if (map === undefined) return {};
  if (!map || typeof map !== "object" || Array.isArray(map)) return `${what}格式不对`;
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(map)) {
    if (typeof value !== "string" || !VAR_RE.test(value)) return `${what} ${key} 带了明文取值；分享链接只接受 \${变量} 引用`;
    out[key] = value;
  }
  return out;
}

function validate(payload: SharePayload): SharePayload | string {
  if (!payload || typeof payload !== "object") return "链接内容为空";
  if (typeof payload.name !== "string" || !NAME_RE.test(payload.name)) return "名称只能用字母、数字、- 与 _";
  if (payload.kind === "mcp") {
    const spec = payload.spec ?? {};
    if (typeof spec.url === "string") {
      if (!/^https:\/\/|^http:\/\/(localhost|127\.0\.0\.1)(:\d+)?(\/|$)/.test(spec.url)) return "远程服务地址只接受 https://，或本机 http://localhost";
      const headers = varsOnly(spec.headers, "header");
      if (typeof headers === "string") return headers;
      return { kind: "mcp", name: payload.name, spec: { url: spec.url, headers } };
    }
    if (typeof spec.command !== "string" || !spec.command.trim()) return "缺少启动命令";
    if (spec.args !== undefined && !(Array.isArray(spec.args) && spec.args.every((arg) => typeof arg === "string"))) return "参数格式不对";
    const env = varsOnly(spec.env, "环境变量");
    if (typeof env === "string") return env;
    return { kind: "mcp", name: payload.name, spec: { command: spec.command, args: spec.args ?? [], env } };
  }
  if (payload.kind === "skills") {
    const def: Partial<Extract<SharePayload, { kind: "skills" }>["definition"]> = payload.definition ?? {};
    if (typeof def.url !== "string" || !/^(https:\/\/|git@)/.test(def.url)) return "技能集合只接受 git 地址（https:// 或 git@），不接受本机路径";
    const paths = def.skill_paths ?? null;
    if (paths !== null && !(Array.isArray(paths) && paths.every((item) => typeof item === "string" && !item.includes("..") && !item.startsWith("/")))) {
      return "技能位置格式不对";
    }
    const rules = def.rules_path ?? null;
    if (rules !== null && (typeof rules !== "string" || rules.includes("..") || rules.startsWith("/"))) return "规则位置格式不对";
    return { kind: "skills", name: payload.name, definition: { url: def.url, skill_paths: paths, rules_path: rules } };
  }
  return "只支持分享 MCP 服务与技能集合";
}
