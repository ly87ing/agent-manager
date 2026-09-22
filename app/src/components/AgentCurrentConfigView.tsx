import { Check, CircleAlert } from "lucide-react";
import type {
  AgentCurrentConfig,
  AgentCurrentConfigStatus,
  AgentRuntimeField,
} from "../lib/observed";
import type { AgentRuntimeSettings } from "../lib/agentDefinitionForm";
import { runtimeComparisons } from "../lib/agentRuntime";
import { cn } from "../lib/utils";

function present(value: unknown): string {
  return value === undefined || value === null || value === "" ? "客户端默认" : String(value);
}

function providerRows(config: AgentCurrentConfig | null | undefined) {
  const aliases = config?.model_aliases;
  const aliasText = [
    aliases?.opus ? `Opus → ${aliases.opus}` : null,
    aliases?.sonnet ? `Sonnet → ${aliases.sonnet}` : null,
    aliases?.haiku ? `Haiku → ${aliases.haiku}` : null,
  ].filter(Boolean).join(" · ");
  const credential = config?.credential;
  const credentialText = credential?.configured
    ? `已配置${credential.kind ? `（${credential.kind}${credential.variable ? ` · ${credential.variable}` : ""}）` : ""}`
    : "使用助手默认登录";
  return [
    ["Provider", present(config?.provider)],
    ["API 地址", present(config?.base_url)],
    ["默认模型", present(config?.model)],
    ...(config?.small_fast_model !== undefined ? [["小任务模型", present(config.small_fast_model)]] : []),
    ...(config?.model_aliases !== undefined ? [["模型映射", aliasText || "客户端默认"]] : []),
    ["请求超时", config?.timeout_ms === undefined || config?.timeout_ms === null ? "客户端默认" : `${config.timeout_ms} ms`],
    ["凭据", credentialText],
  ] as Array<[string, string]>;
}

function stateMessage(status: AgentCurrentConfigStatus | null | undefined): string {
  switch (status?.state) {
    case "missing": return "配置文件尚未创建，客户端当前使用默认值。";
    case "invalid": return "配置文件内容无法解析，已停止判断，避免误导。";
    case "unsafe": return "目标不是普通文件，出于安全考虑未读取。";
    case "unreadable": return "配置文件不可读，请检查文件权限。";
    case "too-large": return "配置文件超过安全读取上限，未读取。";
    case "ready": return "以下内容来自客户端配置文件的只读脱敏回读。";
    default: return "客户端实际配置暂不可读。";
  }
}

/** 以用户能直接核对的字段展示客户端实际值；不暴露内部 JSON 或凭据正文。 */
export function AgentCurrentConfigView({
  status,
  fields,
  savedRuntime,
  showProvider = true,
}: {
  status: AgentCurrentConfigStatus | null | undefined;
  fields: AgentRuntimeField[];
  savedRuntime: AgentRuntimeSettings;
  showProvider?: boolean;
}) {
  const comparisons = runtimeComparisons(fields, savedRuntime, status);
  const currentProviderRows = providerRows(status?.config);
  const readable = status?.state === "ready" || status?.state === "missing";
  const differences = comparisons.filter((item) => item.aligned === false).length;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3 rounded-lg border border-primary/20 bg-primary/5 px-3 py-3">
        <div>
          <p className="text-xs text-muted-foreground">客户端配置文件</p>
          <code className="mt-1 block break-all text-xs font-semibold text-foreground">
            {status?.target ?? status?.file ?? "目标文件暂不可用"}
          </code>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">{stateMessage(status)}</p>
        </div>
        <span className={cn(
          "inline-flex items-center gap-1 rounded-full px-2.5 py-1 text-xs font-medium",
          readable && differences === 0
            ? "bg-success/15 text-success"
            : readable
              ? "bg-warning/15 text-warning"
              : "bg-muted text-muted-foreground",
        )}>
          {readable && differences === 0 ? <Check className="h-3.5 w-3.5" aria-hidden /> : <CircleAlert className="h-3.5 w-3.5" aria-hidden />}
          {!readable ? "无法核对" : differences === 0 ? "运行设置一致" : `${differences} 项未生效`}
        </span>
      </div>

      {showProvider && status?.state === "ready" && (
        <section>
          <h4 className="text-sm font-semibold text-foreground">接入 Provider</h4>
          <dl className="mt-2 grid gap-px overflow-hidden rounded-lg border bg-border sm:grid-cols-2">
            {currentProviderRows.map(([label, value], index) => (
              <div
                key={label}
                className={cn(
                  "min-w-0 bg-background px-3 py-2.5",
                  currentProviderRows.length % 2 === 1 && index === currentProviderRows.length - 1 && "sm:col-span-2",
                )}
              >
                <dt className="text-[11px] text-muted-foreground">{label}</dt>
                <dd className="mt-0.5 break-all text-sm text-foreground">{value}</dd>
              </div>
            ))}
          </dl>
        </section>
      )}

      <section>
        <h4 className="text-sm font-semibold text-foreground">自动化与权限</h4>
        <div className="mt-2 overflow-hidden rounded-lg border">
          <div className="hidden grid-cols-[minmax(0,0.8fr)_minmax(0,1fr)_minmax(0,1fr)] bg-muted/60 text-xs text-muted-foreground sm:grid">
            <span className="px-3 py-2 font-medium">设置项</span>
            <span className="px-3 py-2 font-medium">软件保存值</span>
            <span className="px-3 py-2 font-medium">客户端实际值</span>
          </div>
          {comparisons.map((item) => (
            <div
              key={item.key}
              data-testid={`runtime-comparison-${item.key}`}
              className={cn(
                "grid gap-2 border-t px-3 py-3 text-xs first:border-t-0 sm:grid-cols-[minmax(0,0.8fr)_minmax(0,1fr)_minmax(0,1fr)] sm:gap-0 sm:px-0 sm:py-0",
                item.aligned === false && "bg-warning/5",
              )}
            >
              <strong className="font-medium text-foreground sm:px-3 sm:py-2">{item.label}</strong>
              <span className="break-words text-foreground sm:px-3 sm:py-2">
                <span className="mr-1 text-[11px] text-muted-foreground sm:hidden">软件保存：</span>{item.saved}
              </span>
              <span className={cn("break-words sm:px-3 sm:py-2", item.aligned === false ? "font-medium text-warning" : "text-muted-foreground")}>
                <span className="mr-1 text-[11px] text-muted-foreground sm:hidden">客户端实际：</span>{item.actual}
              </span>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}
