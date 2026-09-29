/**
 * 「粘贴配置」：把 README 里常见的 MCP 配置片段读成表单内容。认这几种写法：
 * `{"mcpServers": {"名字": {...}}}`、`{"名字": {...}}`、`"名字": {...}`（没带外层大括号），
 * 以及只有服务本身的 `{"command": ...}`。只读、不写任何东西；多个服务时取第一个。
 */
export interface PastedServer {
  name: string | null;
  transport: "stdio" | "remote";
  command: string;
  args: string[];
  env: Record<string, string>;
  url: string;
  headers: Record<string, string>;
  /** 给用户看的补充说明（如只取了第一个、请求头改成了变量）。 */
  notes: string[];
}

type Json = Record<string, unknown>;

const isObject = (value: unknown): value is Json => typeof value === "object" && value !== null && !Array.isArray(value);
const isServer = (value: unknown): value is Json =>
  isObject(value) && (typeof value.command === "string" || typeof value.url === "string" || typeof value.serverUrl === "string");
const VAR_RE = /^\$\{[A-Za-z_][A-Za-z0-9_]*\}$/;
const SECRET_NAME_RE = /KEY|TOKEN|SECRET|PASSWORD|PASSWD|CREDENTIAL|AUTH/i;

function parseLoose(text: string): unknown {
  const trimmed = text.trim();
  try {
    return JSON.parse(trimmed);
  } catch {
    // `"名字": {...}` 这种没有外层大括号的片段，去掉结尾逗号后补上再试
    return JSON.parse(`{${trimmed.replace(/,\s*$/, "")}}`);
  }
}

function stringMap(value: unknown): Record<string, string> {
  if (!isObject(value)) return {};
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, String(item)]));
}

export function parsePastedServer(text: string): PastedServer | string {
  let root: unknown;
  try {
    root = parseLoose(text);
  } catch {
    return "读不出来：请粘贴 JSON 格式的配置，例如 {\"mcpServers\": {\"名字\": {\"command\": \"npx\", ...}}}";
  }
  if (!isObject(root)) return "读不出来：配置应该是一个 JSON 对象";
  const container = isObject(root.mcpServers) ? root.mcpServers : isObject(root.mcp) ? root.mcp : isObject(root.servers) ? root.servers : root;
  let name: string | null = null;
  let server: Json;
  const notes: string[] = [];
  if (isServer(container)) {
    server = container;
  } else {
    const entries = Object.entries(container).filter(([, value]) => isServer(value));
    if (entries.length === 0) return "没找到服务：配置里要有 command（本地命令）或 url（远程地址）";
    [name, server] = entries[0] as [string, Json];
    if (entries.length > 1) notes.push(`粘贴的内容里有 ${entries.length} 个服务，只填了第一个 ${name}；其余请分别添加。`);
  }
  const url = typeof server.url === "string" ? server.url : typeof server.serverUrl === "string" ? server.serverUrl : "";
  if (url) {
    const headers: Record<string, string> = {};
    for (const [key, value] of Object.entries(stringMap(server.headers))) {
      if (VAR_RE.test(value)) {
        headers[key] = value;
      } else {
        const variable = key.replace(/[^A-Za-z0-9_]/g, "_").toUpperCase();
        headers[key] = `\${${variable}}`;
        notes.push(`请求头 ${key} 的取值没有填进来，改成了变量 \${${variable}}；取值放进密钥文件。`);
      }
    }
    return { name, transport: "remote", command: "", args: [], env: {}, url, headers, notes };
  }
  const args = Array.isArray(server.args) ? server.args.map(String) : [];
  const env: Record<string, string> = {};
  for (const [key, value] of Object.entries(stringMap(server.env))) {
    // 名字像密钥的明文取值不填进表单，改成同名变量
    if (SECRET_NAME_RE.test(key) && !VAR_RE.test(value)) {
      env[key] = `\${${key}}`;
      notes.push(`${key} 的取值没有填进来，改成了变量 \${${key}}；取值放进密钥文件。`);
    } else {
      env[key] = value;
    }
  }
  return { name, transport: "stdio", command: String(server.command), args, env, url: "", headers: {}, notes };
}
