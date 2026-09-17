import { Download } from "lucide-react";
import type { McpImportCandidate } from "../lib/contract";
import { SERVER_NAME_RE } from "../lib/mcpImport";
import { cn } from "../lib/utils";

/** 候选项的禁用原因：同名冲突或非法名（引擎已排除，此处按当前库视图做防御性双保险）。 */
export function importBlockReason(candidate: McpImportCandidate, existingNames: ReadonlySet<string>): string | null {
  if (!SERVER_NAME_RE.test(candidate.name)) {
    return "名称非法：仅支持字母、数字、短横线与下划线";
  }
  if (existingNames.has(candidate.name)) {
    return "库中已存在同名定义";
  }
  return null;
}

function maskedEntries(map: Record<string, string> | undefined): string {
  const entries = Object.entries(map ?? {});
  if (entries.length === 0) return "无";
  return entries.map(([key, value]) => `${key}=${value}`).join(", ");
}

/**
 * 「发现未导入的服务器」区块：展示各助手现有配置中未受管的 MCP 条目。
 * 导入 = 保存定义到服务库（单级确认，不执行添加/更新动作）；
 * env/header 只展示掩码键清单（<literal> 或 ${VAR} 引用），绝无明文。
 */
export function McpImportCandidates({
  candidates,
  existingNames,
  onImport,
}: {
  candidates: McpImportCandidate[];
  /** 库中已有定义名集合（状态元素 + policy 定义），用于同名冲突禁用。 */
  existingNames: ReadonlySet<string>;
  onImport: (candidate: McpImportCandidate) => void;
}) {
  return (
    <section data-testid="mcp-import-candidates" className="rounded-lg border bg-card p-5 shadow-sm">
      <h3 className="flex items-center gap-2 text-sm font-semibold text-foreground">
        <Download className="h-4 w-4 text-primary" aria-hidden />
        发现未导入的服务器
      </h3>
      <p className="mt-1 text-xs text-muted-foreground">
        以下来自各助手现有配置，尚未纳入服务库。导入仅保存定义（密钥以变量引用形式入库，本工具拿不到明文），不改动助手配置。
      </p>
      {candidates.length === 0 ? (
        <p className="mt-3 text-sm text-muted-foreground">未发现可导入的服务器。</p>
      ) : (
        <ul className="mt-3 space-y-2">
          {candidates.map((candidate) => {
            const reason = importBlockReason(candidate, existingNames);
            return (
              <li
                key={`${candidate.agent}:${candidate.name}`}
                data-testid={`import-candidate-${candidate.name}`}
                className="flex flex-wrap items-center justify-between gap-3 rounded-md border px-3 py-2"
              >
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="font-mono text-xs font-semibold text-foreground">{candidate.name}</span>
                    <span className="rounded border px-1 py-0.5 text-[10px] text-muted-foreground">{candidate.agent}</span>
                    <span className="rounded border border-info/30 bg-info/10 px-1 py-0.5 text-[10px] text-info">
                      {candidate.transport === "remote" ? "远程" : "stdio"}
                    </span>
                    {candidate.previously_managed === true && (
                      <span
                        data-testid={`previously-managed-${candidate.name}`}
                        className="rounded border border-violet/30 bg-violet/10 px-1 py-0.5 text-[10px] text-violet"
                      >
                        此前由本工具管理
                      </span>
                    )}
                  </div>
                  <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
                    {candidate.transport === "remote"
                      ? `headers: ${maskedEntries(candidate.spec.headers)}`
                      : `env: ${maskedEntries(candidate.spec.env)}`}
                  </p>
                  {reason && (
                    <p data-testid={`import-blocked-${candidate.name}`} className="mt-1 text-xs text-warning">
                      {reason}
                    </p>
                  )}
                </div>
                <button
                  type="button"
                  data-testid={`import-button-${candidate.name}`}
                  disabled={reason !== null}
                  onClick={() => onImport(candidate)}
                  className={cn(
                    "inline-flex items-center gap-1 rounded border px-2.5 py-1 text-xs",
                    reason !== null
                      ? "cursor-not-allowed border bg-muted text-muted-foreground"
                      : "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15",
                  )}
                >
                  <Download className="h-3.5 w-3.5" aria-hidden /> 导入
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
