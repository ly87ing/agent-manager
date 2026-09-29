import { useEffect, useMemo, useRef, useState } from "react";
import { Search } from "lucide-react";
import { toast } from "sonner";
import { buildAccounts } from "../lib/accounts";
import { trayUsageSummary } from "../lib/agentUsage";
import type { StatusData } from "../lib/contract";
import { useEngineQuery, useRefreshUpstream } from "../lib/queries";
import { publishTrayAccounts, type TrayAccount } from "../lib/trayActions";
import { cn } from "../lib/utils";
import { matchesKeyword } from "./SearchInput";

export interface CommandItem {
  id: string;
  label: string;
  /** 右侧说明：分类、启动命令或动作结果。 */
  hint?: string;
  /** 额外匹配词（英文名、命令名、目录等）。 */
  keywords?: string;
  group: string;
  run: () => void;
}

const PAGES: { key: string; label: string; keywords: string }[] = [
  { key: "accounts", label: "账号", keywords: "accounts 账号 claude codex 启动命令 额度" },
  { key: "skills", label: "技能", keywords: "skills 技能" },
  { key: "mcp", label: "MCP", keywords: "mcp 服务器" },
  { key: "rules-hooks", label: "规则", keywords: "rules hooks 规则" },
  { key: "usage", label: "用量", keywords: "usage 用量 token 花费 成本" },
  { key: "settings", label: "设置", keywords: "settings 偏好" },
  // 设置里的二级页面也要搜得到
  { key: "takeover", label: "导入现有配置", keywords: "takeover 接管 发现 导入 扫描" },
  { key: "runs", label: "操作记录", keywords: "runs 任务 日志 记录" },
  { key: "settings:secrets", label: "密钥", keywords: "secrets 密钥 变量 token api key 凭据" },
  { key: "commands", label: "命令", keywords: "aliases alias 别名 启动命令 终端快捷命令 时区" },
  { key: "sources", label: "来源", keywords: "sources 来源 集合 仓库 规则来源 添加 停用 移除 agent-skills" },
];

/**
 * 命令面板（⌘K）：一个入口搜到页面、账号与资源，键盘直接完成。⌘⇧K 只列账号。
 *
 * 选中账号即复制它的启动命令；面板本身不写任何配置。同一份账号列表发给托盘菜单，
 * 托盘点中某个账号会带着精确目标重新打开面板，按 Enter 复制。
 */
export function CommandPalette({
  open,
  scope = "all",
  initialKeyword,
  initialItemId,
  onClose,
  onNavigate,
  onOpenAgentSkills,
}: {
  open: boolean;
  scope?: "all" | "quick";
  /** 托盘目标会带精确查询，打开后直接把焦点落在对应账号。 */
  initialKeyword?: string | null;
  initialItemId?: string | null;
  onClose: () => void;
  onNavigate: (page: string) => void;
  /** 选中某个账号的技能时，跳到技能页并只看它。 */
  onOpenAgentSkills: (agentId: string) => void;
}) {
  const [keyword, setKeyword] = useState("");
  const [targetItemId, setTargetItemId] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const [usageNow, setUsageNow] = useState(() => new Date());
  const inputRef = useRef<HTMLInputElement>(null);
  const refreshUpstream = useRefreshUpstream();

  useEffect(() => {
    // 不重扫会话文件；只让托盘里的额度在窗口重置后消失、过旧后标明。
    const timer = window.setInterval(() => setUsageNow(new Date()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  // 托盘要在窗口隐藏时也有账号列表，这三份状态保持热缓存（与账号页共享请求）
  const agents = useEngineQuery<StatusData>("agents:status", [], { enabled: true });
  const profiles = useEngineQuery<StatusData>("profiles:status", [], { enabled: true });
  const aliases = useEngineQuery<StatusData>("aliases:status", [], { enabled: true });
  const skills = useEngineQuery<StatusData>("skills:status", [], { enabled: open });
  const mcp = useEngineQuery<StatusData>("mcp:status", [], { enabled: open });

  const groups = useMemo(() => {
    const aliasValues = Object.fromEntries(
      (aliases.data?.ok ? aliases.data.data.aliases ?? [] : [])
        .filter((item) => item.value !== null)
        .map((item) => [item.name, item.value as string]),
    );
    return agents.data?.ok
      ? buildAccounts(agents.data.data.elements, profiles.data?.ok ? profiles.data.data.elements : [], aliasValues)
      : [];
  }, [agents.data, profiles.data, aliases.data]);

  const trayAccounts = useMemo<TrayAccount[]>(() => groups.flatMap((group) => group.accounts.map((account) => ({
    agentId: group.agentId,
    agentName: group.label,
    accountId: account.elementId,
    accountName: account.isMain ? "主账号" : account.name,
    command: account.command,
    usageSummary: trayUsageSummary(account.usage, usageNow),
  }))), [groups, usageNow]);

  useEffect(() => {
    // 三份状态都成功后才替换原生菜单，避免瞬时读取失败清空上一次可用菜单
    if (agents.data?.ok && profiles.data?.ok && aliases.data?.ok) void publishTrayAccounts(trayAccounts);
  }, [agents.data?.ok, profiles.data?.ok, aliases.data?.ok, trayAccounts]);

  useEffect(() => {
    if (open) {
      setKeyword(initialKeyword ?? "");
      setTargetItemId(initialItemId ?? null);
      setActive(0);
      window.setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [initialItemId, initialKeyword, open]);

  const items = useMemo<CommandItem[]>(() => {
    const out: CommandItem[] = PAGES.map((page) => ({
      id: `page:${page.key}`,
      label: page.label,
      hint: "页面",
      keywords: page.keywords,
      group: "前往",
      run: () => onNavigate(page.key),
    }));
    for (const group of groups) {
      for (const account of group.accounts) {
        const name = account.isMain ? "主账号" : account.name;
        const command = account.command;
        out.push({
          id: `account:${account.elementId}`,
          label: `${group.label} · ${name}`,
          hint: command ? `复制 ${command}` : "没有启动命令",
          keywords: `${group.agentId} ${command ?? ""} ${account.configDir} ${account.apiHost ?? ""} 账号`,
          group: "账号",
          run: () => {
            if (!command) {
              onNavigate("accounts");
              return;
            }
            void navigator.clipboard.writeText(command)
              .then(() => toast.success(`已复制启动命令 ${command}`))
              .catch(() => toast.error("复制失败，请到账号页查看启动命令"));
          },
        });
        const skillsId = account.isMain ? group.agentId : account.elementId.replace(/^profiles:/, "");
        out.push({
          id: `account-skills:${skillsId}`,
          label: `${group.label} · ${name} 的技能`,
          hint: "技能",
          keywords: `${skillsId} skills`,
          group: "资源",
          run: () => onOpenAgentSkills(skillsId),
        });
      }
    }
    for (const element of skills.data?.ok ? skills.data.data.elements : []) {
      const id = element.id.replace(/^skill:/, "");
      if (id.includes("/")) continue;
      out.push({ id: `skill:${id}`, label: id, hint: "技能集合", keywords: `${id} skills`, group: "资源", run: () => onNavigate("skills") });
    }
    for (const element of mcp.data?.ok ? mcp.data.data.elements : []) {
      const id = element.id.replace(/^mcp:/, "");
      out.push({ id: `mcp:${id}`, label: id, hint: "MCP 服务", keywords: `${id} mcp`, group: "资源", run: () => onNavigate("mcp") });
    }
    out.push({
      id: "action:import-link",
      label: "从分享链接导入…",
      hint: "MCP 服务或技能集合",
      keywords: "import share link 分享 导入 链接",
      group: "动作",
      run: () => onNavigate("import-link"),
    });
    out.push({
      id: "action:refresh-upstream",
      label: "检查更新",
      hint: "对比官方最新版本",
      keywords: "update upstream 版本",
      group: "动作",
      run: () => refreshUpstream.mutate(undefined),
    });
    return out;
  }, [groups, skills.data, mcp.data, refreshUpstream, onNavigate, onOpenAgentSkills]);

  const filtered = useMemo(() => {
    const scoped = scope === "quick" ? items.filter((item) => item.group === "账号") : items;
    return targetItemId
      ? scoped.filter((item) => item.id === targetItemId)
      : scoped.filter((item) => matchesKeyword(keyword, [item.label, item.hint, item.keywords]));
  }, [items, keyword, scope, targetItemId]);

  useEffect(() => {
    setActive((prev) => (prev >= filtered.length ? 0 : prev));
  }, [filtered.length]);

  if (!open) return null;

  const runItem = (item: CommandItem) => {
    item.run();
    onClose();
  };

  const grouped = filtered.reduce<Record<string, CommandItem[]>>((acc, item) => {
    (acc[item.group] ??= []).push(item);
    return acc;
  }, {});
  let renderIndex = -1;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-slate-900/40 p-4 pt-[12vh] animate-fade-in"
      onClick={(event) => event.target === event.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={scope === "quick" ? "账号" : "命令面板"}
        data-testid="command-palette"
        className="w-full max-w-xl overflow-hidden rounded-xl border bg-card shadow-xl animate-zoom-in-95"
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setActive((prev) => (filtered.length === 0 ? 0 : (prev + 1) % filtered.length));
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            setActive((prev) => (filtered.length === 0 ? 0 : (prev - 1 + filtered.length) % filtered.length));
          } else if (event.key === "Enter") {
            event.preventDefault();
            const item = filtered[active];
            if (item) runItem(item);
          } else if (event.key === "Escape") {
            event.preventDefault();
            onClose();
          }
        }}
      >
        <div className="flex items-center gap-2 border-b px-4 py-3">
          <Search className="h-4 w-4 text-muted-foreground" aria-hidden />
          <input
            ref={inputRef}
            value={keyword}
            onChange={(event) => {
              setTargetItemId(null);
              setKeyword(event.target.value);
            }}
            data-testid="command-palette-input"
            aria-label={scope === "quick" ? "搜索账号" : "搜索页面、账号与资源"}
            placeholder={scope === "quick" ? "搜索账号或启动命令…" : "搜索页面、账号、技能、MCP…"}
            className="w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
        </div>
        <div className="max-h-[50vh] overflow-y-auto p-2">
          {filtered.length === 0 ? (
            <p className="px-3 py-6 text-center text-sm text-muted-foreground">
              {scope === "quick" ? "没有匹配的账号。" : "没有匹配的页面、账号或资源。"}
            </p>
          ) : (
            Object.entries(grouped).map(([group, groupItems]) => (
              <div key={group} className="mb-2 last:mb-0">
                <p className="px-3 py-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">{group}</p>
                {groupItems.map((item) => {
                  renderIndex += 1;
                  const index = renderIndex;
                  return (
                    <button
                      key={item.id}
                      type="button"
                      data-testid={`command-item-${item.id}`}
                      onMouseEnter={() => setActive(index)}
                      onClick={() => runItem(item)}
                      className={cn(
                        "flex w-full items-center justify-between gap-3 rounded-md px-3 py-2 text-left text-sm",
                        index === active ? "bg-accent text-foreground" : "text-foreground hover:bg-accent/60",
                      )}
                    >
                      <span className="truncate">{item.label}</span>
                      {item.hint && <span className="shrink-0 text-xs text-muted-foreground">{item.hint}</span>}
                    </button>
                  );
                })}
              </div>
            ))
          )}
        </div>
        <div className="border-t px-4 py-2 text-[11px] text-muted-foreground">↑↓ 选择 · Enter 执行 · Esc 关闭</div>
      </div>
    </div>
  );
}
