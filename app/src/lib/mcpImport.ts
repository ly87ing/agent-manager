/**
 * MCP 导入候选 → 库定义的纯转换（冻结语义）：
 * - env 值一律转为 ${<ENV_KEY>}（键含非法占位符字符时下划线化，键本身不变）；
 * - headers 值：已是 ${VAR} 引用的保留，<literal> 的一律转为 ${<HEADER_NAME大写下划线化>}；
 * - remote 保留 url；agents 固定为 [<来源 agent>]。
 * 输入只有引擎掩码后的 spec（<literal> / ${VAR}），前端自始至终拿不到明文；
 * 转换结果走 policy:set 保存期完整校验，不绕过任何现有闸门。
 */

import type { McpImportCandidate } from "./contract";

/** server 名校验（与引擎 is_valid_server_name 一致）。 */
export const SERVER_NAME_RE = /^[A-Za-z0-9_-]+$/;

/** 引擎「同名条目」fail-closed 错误/blocked_reason 的稳定片段（显式接管通道的触发标识；
 * 覆盖「已存在非××同名条目…拒绝覆盖/拒绝自动删除」与「存在外部非××同名条目」全部变体）。 */
export const MCP_UNMANAGED_CONFLICT_MARKER = "同名条目";

const PLACEHOLDER_RE = /^\$\{[A-Za-z_][A-Za-z0-9_]*\}$/;

/** header 名 → 占位符变量名：大写下划线化，数字开头补 _ 前缀。 */
export function headerToVarName(name: string): string {
  let out = "";
  for (const ch of name) {
    out += /[A-Za-z0-9]/.test(ch) ? ch.toUpperCase() : "_";
  }
  if (/^[0-9]/.test(out)) out = `_${out}`;
  return out;
}

/** env 键 → 占位符变量名：合法标识符原样保留，否则下划线化（数字开头补 _ 前缀）。 */
function envToVarName(key: string): string {
  if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) return key;
  let out = "";
  for (const ch of key) {
    out += /[A-Za-z0-9_]/.test(ch) ? ch : "_";
  }
  if (/^[0-9]/.test(out)) out = `_${out}`;
  return out;
}

export function candidateToDefinition(candidate: McpImportCandidate): Record<string, unknown> {
  if (candidate.transport === "remote") {
    const headers: Record<string, string> = {};
    for (const [name, value] of Object.entries(candidate.spec.headers ?? {})) {
      headers[name] = PLACEHOLDER_RE.test(value) ? value : `\${${headerToVarName(name)}}`;
    }
    return { url: candidate.spec.url, headers, agents: [candidate.agent] };
  }
  const env: Record<string, string> = {};
  for (const key of Object.keys(candidate.spec.env ?? {})) {
    env[key] = `\${${envToVarName(key)}}`;
  }
  return {
    command: candidate.spec.command,
    args: candidate.spec.args ?? [],
    env,
    agents: [candidate.agent],
  };
}
