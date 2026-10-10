import { X } from "lucide-react";
import { toast } from "sonner";
import type { Account, AgentAccounts } from "../lib/accounts";
import type { StatusData } from "../lib/contract";
import { useExecutions } from "../lib/execution";
import { mcpFillArgs, mcpFillRows } from "../lib/mcpFill";
import type { OfficialAuthStatus } from "../lib/observed";
import { useEngineQuery } from "../lib/queries";
import { rulesFillArgs, rulesFillRows } from "../lib/rulesFill";
import { fillRows } from "../lib/skillFill";
import { useBatchApply } from "../lib/useBatchApply";
import { cn } from "../lib/utils";
import { AgentIcon } from "./AgentIcon";

type Tone = "ok" | "warn" | "muted";
interface Item {
  key: string;
  label: string;
  tone: Tone;
  text: string;
  action?: { label: string; run: () => void };
}

const DOT: Record<Tone, string> = { ok: "bg-success", warn: "bg-warning", muted: "bg-muted-foreground/50" };

/**
 * 一个账号的准备情况：登录、启动命令、技能、MCP、规则逐项核对，缺什么就地补。
 * 判断全部复用各页已有的规则（补齐只补在用的、停用的不动；规则只给还没有文件的从主号复制）。
 */
export function AccountReadiness({
  account,
  group,
  auth,
  isNew,
  onCopyLogin,
  onAddLaunch,
  onProbeAuth,
  onClose,
}: {
  account: Account;
  group: AgentAccounts;
  auth: OfficialAuthStatus | "pending" | null;
  /** 刚建好的账号：标题这样说。 */
  isNew: boolean;
  onCopyLogin: () => void;
  loginShellLabel?: string;
  onAddLaunch: () => void;
  onProbeAuth: () => void;
  onClose: () => void;
}) {
  const skills = useEngineQuery<StatusData>("skills:status");
  const mcp = useEngineQuery<StatusData>("mcp:status");
  const rules = useEngineQuery<StatusData>("rules:status");
  const batchApply = useBatchApply();
  const { execute } = useExecutions();

  const key = account.isMain ? group.agentId : account.elementId.replace(/^profiles:/, "");
  const skillRows = skills.data?.ok ? fillRows(skills.data.data.elements ?? [], [key]) : [];
  const mcpRows = mcp.data?.ok ? mcpFillRows(mcp.data.data.elements ?? [], mcp.data.data.targets ?? [], [key]) : [];
  const ruleId = account.isMain ? `rules:${group.kind}_global` : `rules:profile:${key}`;
  const ruleElements = rules.data?.ok ? rules.data.data.elements ?? [] : [];
  const ruleElement = ruleElements.find((element) => element.id === ruleId);
  const ruleFill = rulesFillRows(ruleElements).find((row) => row.elementId === ruleId);
  const hasRules = typeof (ruleElement?.observed as { content?: unknown } | undefined)?.content === "string";

  /** 逐条执行（MCP、规则），单条失败不拦其余，最后汇总。 */
  const runEach = async (label: string, rows: { label: string; command: string; args: string[] }[]) => {
    let failed = 0;
    for (const row of rows) {
      const envelope = await execute({ command: row.command, label: row.label, args: row.args, silent: true });
      if (!envelope?.ok) failed += 1;
    }
    (failed ? toast.error : toast.success)(`${label}：成功 ${rows.length - failed}，失败 ${failed}${failed ? "，详见任务记录" : ""}`);
  };

  const items: Item[] = [];
  const unsafe = auth !== "pending" && Boolean(auth?.mixed || auth?.isolation_blocked || account.blockedReason || account.mainHasThirdParty);
  if (unsafe) {
    items.push({ key: "login", label: "账号检查", tone: "warn", text: (typeof auth === "object" && auth?.reason) || account.blockedReason || "登录与 API 设置混在同一个目录，已阻止操作" });
  } else if (account.type === "official") {
    items.push(
      auth === "pending"
        ? { key: "login", label: "登录", tone: "muted", text: "检查中…" }
        : auth?.state === "logged-in"
          ? { key: "login", label: "登录", tone: "ok", text: auth.account?.email ? `已登录 ${auth.account.email}` : "已登录" }
          : auth?.state === "unknown"
            ? { key: "login", label: "登录", tone: "muted", text: "本机记录说明不了", action: { label: "确认", run: onProbeAuth } }
            : {
                key: "login",
                label: "登录",
                tone: "warn",
                text: "还没登录",
                action: {
                  label: "复制登录命令",
                  run: onCopyLogin,
                },
              },
    );
  }
  items.push(
    unsafe
      ? { key: "launch", label: "启动命令", tone: "warn", text: "账号检查通过后才能使用" }
      : account.isMain
      ? { key: "launch", label: "启动命令", tone: "ok", text: "从账号行复制受控启动命令" }
      : account.command
        ? { key: "launch", label: "启动命令", tone: "ok", text: account.command }
        : { key: "launch", label: "启动命令", tone: "warn", text: "还没有", action: { label: "写入启动命令", run: onAddLaunch } },
  );
  items.push(
    skillRows.length === 0
      ? { key: "skills", label: "技能", tone: "ok", text: "在用的技能都有了" }
      : {
          key: "skills",
          label: "技能",
          tone: "warn",
          text: `缺 ${skillRows.length} 个在用的技能`,
          action: { label: "补齐", run: () => void batchApply.applyBatch({ actionLabel: "补齐", rows: skillRows }) },
        },
  );
  items.push(
    mcpRows.length === 0
      ? { key: "mcp", label: "MCP", tone: "ok", text: "在用的服务都有了" }
      : {
          key: "mcp",
          label: "MCP",
          tone: "warn",
          text: `缺 ${mcpRows.length} 个在用的服务`,
          action: {
            label: "补齐",
            run: () =>
              void runEach(
                "补齐 MCP",
                mcpRows.map((row) => ({ label: `补齐 ${row.name}`, command: "mcp:apply", args: mcpFillArgs(row) })),
              ),
          },
        },
  );
  items.push(
    hasRules
      ? { key: "rules", label: "规则", tone: "ok", text: "已有规则文件" }
      : ruleFill
        ? {
            key: "rules",
            label: "规则",
            tone: "warn",
            text: "还没有规则文件",
            action: { label: "从主号补齐", run: () => void runEach("补齐规则", [{ label: `补齐 ${ruleFill.name}`, command: "rules:apply", args: rulesFillArgs(ruleFill) }]) },
          }
        : { key: "rules", label: "规则", tone: "muted", text: "还没有规则文件（主号也没有）" },
  );

  const done = items.filter((item) => item.tone === "ok").length;
  return (
    <section data-testid={`account-readiness-${account.elementId}`} className="overflow-clip rounded-lg border bg-card" aria-label="准备情况">
      <div className="flex items-center gap-2 border-b bg-muted px-4 py-2 text-xs">
        <AgentIcon kind={group.kind} />
        <span className="font-medium text-foreground">{isNew ? `新账号「${account.name}」的准备情况` : `「${account.name}」的准备情况`}</span>
        <span className="text-muted-foreground">
          {done} / {items.length} 项就绪
        </span>
        <button type="button" aria-label="关闭准备情况" onClick={onClose} className="ml-auto rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground">
          <X className="h-3.5 w-3.5" aria-hidden />
        </button>
      </div>
      <ul className="divide-y">
        {items.map((item) => (
          <li key={item.key} data-testid={`readiness-${item.key}`} className="flex items-center gap-4 px-4 py-2 text-sm">
            <span className="w-20 shrink-0 text-muted-foreground">{item.label}</span>
            <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", DOT[item.tone])} aria-hidden />
            <span className={cn("min-w-0 flex-1 truncate", item.tone === "warn" && "text-warning")}>{item.text}</span>
            {item.action && (
              <button
                type="button"
                data-testid={`readiness-action-${item.key}`}
                disabled={batchApply.executing}
                onClick={item.action.run}
                className="whitespace-nowrap rounded-md border border-primary/30 bg-primary/10 px-2 py-0.5 text-xs text-primary hover:bg-primary/15 disabled:opacity-40"
              >
                {item.action.label}
              </button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
