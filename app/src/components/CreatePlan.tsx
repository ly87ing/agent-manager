import { ChevronRight } from "lucide-react";
import { useState } from "react";
import type { CreatePreview } from "../lib/useAccountOps";
import { cn } from "../lib/utils";

/**
 * 新建账号前逐条说清点「创建」会做什么：建哪个目录、每个文件写什么、终端里加哪条命令、
 * 链接多少技能，以及不会动什么。文件内容来自引擎的只读预览，可以就地展开核对。
 */
export interface PlanFacts {
  configDir: string;
  /** 主账号目录（~/.claude 或 ~/.codex）。 */
  mainDir: string;
  /** 完整启动命令（alias 的取值）。 */
  command: string;
  commandName: string;
  api: { baseUrl: string; model: string; tokenVar: string; keyFile: string | null } | null;
  /** 在用、会链接到新账号的技能数。 */
  skills: number;
  /** 其他同类账号上在用、主账号没有、会另外复制过来的 MCP 服务。 */
  extraMcp: string[];
  /** 官方账号建好后的登录命令。 */
  loginCommand: string | null;
}

const SEED_TEXT: Record<string, string> = {
  permissions: "权限规则",
  statusline: "状态栏",
  hooks: "Hooks",
};

function FileStep({ title, path, content, children }: { title: string; path: string; content: string; children?: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <li className="space-y-1">
      <div className="flex items-start gap-1">
        <span className="min-w-0 flex-1">
          {title} <code className="font-mono text-[11px]">{path}</code>
          {children && <span className="block text-muted-foreground">{children}</span>}
        </span>
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          className="inline-flex shrink-0 items-center text-primary hover:underline"
        >
          <ChevronRight className={cn("h-3 w-3 transition-transform", open && "rotate-90")} aria-hidden />
          {open ? "收起" : "看内容"}
        </button>
      </div>
      {open && <pre className="max-h-48 overflow-auto rounded bg-muted p-2 font-mono text-[11px]">{content}</pre>}
    </li>
  );
}

export function CreatePlan({ preview, facts }: { preview: CreatePreview; facts: PlanFacts }) {
  const seeds = preview.config.seed_results ?? [];
  const copied = (item: string) => seeds.some((row) => row.item === item && row.state === "copied");
  const carried = Object.keys(SEED_TEXT).filter(copied).map((item) => SEED_TEXT[item]);
  const cleared = (() => {
    try {
      const env = (JSON.parse(preview.config.content) as { env?: Record<string, string> }).env ?? {};
      return Object.values(env).filter((value) => value === "").length;
    } catch {
      return 0;
    }
  })();
  const extraFiles = preview.config.files ?? [];
  const rulesFile = extraFiles.find((file) => /(CLAUDE|AGENTS)\.md$/.test(file.path));
  const mcpFile = extraFiles.find((file) => file.path.endsWith(".claude.json") || file.path.endsWith("config.toml"));
  const mcpDetail = seeds.find((row) => row.item === "mcp" && row.state === "copied")?.detail;
  const aliasLine = `alias ${facts.commandName}='${facts.command}'`;
  const aliasFiles = preview.alias.files.map((file) => file.path.replace(/^.*\//, "~/"));

  return (
    <section data-testid="account-create-plan" className="rounded-md border p-3 text-xs">
      <p className="mb-2 font-medium">点「创建」后会：</p>
      <ol className="list-decimal space-y-2 pl-4">
        <li>
          新建目录 <code className="font-mono text-[11px]">{facts.configDir}</code>
          <span className="block text-muted-foreground">这个账号的登录、会话和设置都只放在这里。</span>
        </li>
        <FileStep title="写入" path={preview.config.path} content={preview.config.content}>
          {[
            facts.api ? `接口 ${facts.api.baseUrl}${facts.api.model ? `、模型 ${facts.api.model}` : ""}、API Key（取自 ${facts.api.tokenVar}）` : null,
            carried.length ? `从主账号带上${carried.join("、")}` : null,
            cleared ? `置空 ${cleared} 个凭据相关变量，防止终端里导出的令牌顶替这个账号` : null,
          ].filter(Boolean).join("；")}
        </FileStep>
        {rulesFile && (
          <FileStep title="复制主账号的规则到" path={rulesFile.path} content={rulesFile.content} />
        )}
        {mcpFile && (
          <FileStep title="写入 MCP 服务" path={mcpFile.path} content={mcpFile.content}>
            {mcpDetail}
          </FileStep>
        )}
        {facts.extraMcp.length > 0 && (
          <li>另从其他账号复制在用的 MCP：{facts.extraMcp.join("、")}</li>
        )}
        {facts.skills > 0 && (
          <li>
            链接 {facts.skills} 个在用的技能到 <code className="font-mono text-[11px]">{facts.configDir}/skills</code>
          </li>
        )}
        {facts.api?.keyFile && (
          <li>
            把 API Key 存进 <code className="font-mono text-[11px]">{facts.api.keyFile}</code> 的 <code className="font-mono">{facts.api.tokenVar}</code>（本工具数据库不存取值）
          </li>
        )}
        <li>
          在 {aliasFiles.join("、")} 的 Agent Manager 区块加一行启动命令：
          <code className="mt-1 block break-all rounded bg-muted p-1.5 font-mono text-[11px]">{aliasLine}</code>
          <span className="text-muted-foreground">新开终端后输入 <code className="font-mono">{facts.commandName}</code> 启动。</span>
        </li>
        <li>在本工具里登记这个账号（只记名称、目录和启动命令名）。</li>
      </ol>
      <p className="mt-2 text-muted-foreground">
        不会改动主账号 <code className="font-mono">{facts.mainDir}</code>、它的登录，以及 {aliasFiles.join("、")} 里你自己写的内容。
        {facts.loginCommand && (
          <>
            {" "}建好后还要登录一次：在账号行点「登录」复制
            <code className="mx-1 break-all font-mono">{facts.loginCommand}</code>到终端运行。
          </>
        )}
      </p>
    </section>
  );
}
