import type {
  AgentCurrentConfig,
  AgentCurrentConfigStatus,
  AgentRuntimeField,
} from "../lib/observed";
import { runtimeActualItems } from "../lib/agentRuntime";
import { cn } from "../lib/utils";

function present(value: unknown): string {
  return value === undefined || value === null || value === "" ? "客户端默认" : String(value);
}

function credentialText(config: AgentCurrentConfig | null | undefined): string {
  const credential = config?.credential;
  if (!credential?.configured) return "使用助手默认登录";
  const detail = `已配置${credential.kind ? `（${credential.kind}${credential.variable ? ` · ${credential.variable}` : ""}）` : ""}`;
  // Claude 的令牌是明文写在配置里的：说明它是否已保存为某个变量，切换前是否需要处理
  if (credential.saved_var) return `${detail}，与变量 ${credential.saved_var} 一致`;
  if (credential.kind === "token" || credential.kind === "api_key") return `${detail}，令牌还没有保存为变量`;
  return detail;
}

function providerRows(config: AgentCurrentConfig | null | undefined) {
  const aliases = config?.model_aliases;
  const aliasText = [
    aliases?.opus ? `Opus → ${aliases.opus}` : null,
    aliases?.sonnet ? `Sonnet → ${aliases.sonnet}` : null,
    aliases?.haiku ? `Haiku → ${aliases.haiku}` : null,
  ].filter(Boolean).join(" · ");
  return [
    ["Provider", present(config?.provider)],
    ["API 地址", present(config?.base_url)],
    ["默认模型", present(config?.model)],
    ...(config?.small_fast_model !== undefined ? [["小任务模型", present(config.small_fast_model)]] : []),
    ...(config?.model_aliases !== undefined ? [["模型映射", aliasText || "客户端默认"]] : []),
    ["请求超时", config?.timeout_ms === undefined || config?.timeout_ms === null ? "客户端默认" : `${config.timeout_ms} ms`],
    ["凭据", credentialText(config)],
  ] as Array<[string, string]>;
}

function stateMessage(status: AgentCurrentConfigStatus | null | undefined): string {
  switch (status?.state) {
    case "missing": return "配置文件尚未创建，客户端当前使用默认值。";
    case "invalid": return "配置文件内容无法解析，已停止读取；修好之前本工具不会写入它。";
    case "unsafe": return "目标不是普通文件（或是断开的链接），出于安全考虑未读取。";
    case "unreadable": return "配置文件不可读，请检查文件权限。";
    case "too-large": return "配置文件超过安全读取上限，未读取。";
    case "ready": return "以下内容来自客户端配置文件的只读脱敏回读，就是当前生效的值。";
    default: return "客户端实际配置暂不可读。";
  }
}

/** 以用户能直接核对的字段展示客户端实际值；不暴露内部 JSON 或凭据正文。 */
export function AgentCurrentConfigView({
  status,
  fields,
  showProvider = true,
}: {
  status: AgentCurrentConfigStatus | null | undefined;
  fields: AgentRuntimeField[];
  showProvider?: boolean;
}) {
  const items = runtimeActualItems(fields, status);
  const currentProviderRows = providerRows(status?.config);

  return (
    <div className="space-y-4">
      <div className="rounded-lg border border-primary/20 bg-primary/5 px-3 py-3">
        <p className="text-xs text-muted-foreground">客户端配置文件</p>
        <code className="mt-1 block break-all text-xs font-semibold text-foreground">
          {status?.target ?? status?.file ?? "目标文件暂不可用"}
        </code>
        <p className="mt-1 text-xs leading-5 text-muted-foreground">{stateMessage(status)}</p>
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
        <dl className="mt-2 grid gap-px overflow-hidden rounded-lg border bg-border sm:grid-cols-2">
          {items.map((item) => (
            <div key={item.key} data-testid={`runtime-actual-${item.key}`} className="min-w-0 bg-background px-3 py-2.5">
              <dt className="text-[11px] text-muted-foreground">{item.label}</dt>
              <dd className="mt-0.5 break-words text-sm text-foreground">{item.actual}</dd>
            </div>
          ))}
        </dl>
      </section>
    </div>
  );
}
