import { t, useLanguage } from "../lib/i18n";
import { useEffect, useMemo, useRef, useState } from "react";
import { Search } from "lucide-react";
import type { StatusData } from "../lib/contract";
import { useEngineQuery, useRefreshUpstream } from "../lib/queries";
import { cn } from "../lib/utils";
import { matchesKeyword } from "./SearchInput";

export interface CommandItem {
  id: string;
  label: string;
  /** 右侧说明：分类或动作结果。 */
  hint?: string;
  /** 额外匹配词（英文名、命令名、目录等）。 */
  keywords?: string;
  group: string;
  run: () => void;
}

const PAGES: { key: string; label: string; keywords: string }[] = [
  { key: "skills", get label() { return t("技能"); }, keywords: "skills 技能" },
  { key: "mcp", label: "MCP", keywords: "mcp 服务器" },
  { key: "rules", get label() { return t("规则"); }, keywords: "rules 规则" },
  { key: "agents", get label() { return t("助手"); }, keywords: "agent agents 助手 只读 目录 扩展目标" },
  { key: "settings", get label() { return t("设置"); }, keywords: "settings 偏好" },
  // 设置里的二级页面也要搜得到
  { key: "runs", get label() { return t("操作记录"); }, keywords: "runs 任务 日志 记录" },
  { key: "settings:secrets", get label() { return t("MCP 密钥"); }, keywords: "secrets 密钥 变量 token api key 凭据" },
  { key: "settings:launch", get label() { return t("启动配置"); }, keywords: "launch timezone alias shell 时区 别名 启动 la sg" },
  { key: "settings:tips", get label() { return t("更多技巧"); }, keywords: "tips 技巧 分享 多账号 独立环境 别名 alias shell" },
  { key: "programs", get label() { return t("依赖程序"); }, keywords: "programs 程序 cli 命令行 依赖 npm npx uvx 安装 升级 版本" },
  { key: "sources", get label() { return t("来源"); }, keywords: "sources 来源 集合 仓库 规则来源 添加 更新 版本 停用 移除 未纳管 纳入 接管 扫描 agent-skills" },
];

/**
 * 命令面板（⌘K / ⌘⇧K）：搜索页面和扩展资源。
 *
 * Queries run only while open; the palette never writes configuration.
 */
export function CommandPalette({
  open,
  scope = "all",
  initialKeyword,
  initialItemId,
  onClose,
  onNavigate,
}: {
  open: boolean;
  scope?: "all" | "quick";
  /** 精确查询可将焦点落在对应资源。 */
  initialKeyword?: string | null;
  initialItemId?: string | null;
  onClose: () => void;
  onNavigate: (page: string) => void;
  /** 选中某个账号的技能时，跳到技能页并只看它。 */
  onOpenAgentSkills: (agentId: string) => void;
}) {
  const language = useLanguage();
  const [keyword, setKeyword] = useState("");
  const [targetItemId, setTargetItemId] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const refreshUpstream = useRefreshUpstream();

  const skills = useEngineQuery<StatusData>("skills:status", [], { enabled: open });
  const mcp = useEngineQuery<StatusData>("mcp:status", [], { enabled: open });

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
      hint: t("页面"),
      keywords: page.keywords,
      group: t("前往"),
      run: () => onNavigate(page.key),
    }));
    for (const element of skills.data?.ok ? skills.data.data.elements : []) {
      const id = element.id.replace(/^skill:/, "");
      if (id.includes("/")) continue;
      out.push({ id: `skill:${id}`, label: id, hint: t("技能来源"), keywords: `${id} skills`, group: t("资源"), run: () => onNavigate("skills") });
    }
    for (const element of mcp.data?.ok ? mcp.data.data.elements : []) {
      const id = element.id.replace(/^mcp:/, "");
      out.push({ id: `mcp:${id}`, label: id, hint: t("MCP 服务"), keywords: `${id} mcp`, group: t("资源"), run: () => onNavigate("mcp") });
    }
    out.push({
      id: "action:import-link",
      label: t("从分享链接导入…"),
      hint: t("MCP 服务或技能来源"),
      keywords: "import share link 分享 导入 链接",
      group: t("动作"),
      run: () => onNavigate("import-link"),
    });
    out.push({
      id: "action:refresh-upstream",
      label: t("检查更新"),
      hint: t("对比官方最新版本"),
      keywords: "update upstream 版本",
      group: t("动作"),
      run: () => refreshUpstream.mutate(undefined),
    });
    return out;
  }, [skills.data, mcp.data, refreshUpstream, onNavigate, language]);

  const filtered = useMemo(() => {
    const scoped = items;
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
        aria-label={t("命令面板")}
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
              setActive(0);
              setKeyword(event.target.value);
            }}
            data-testid="command-palette-input"
            aria-label={t("搜索页面与资源")}
            placeholder={t("搜索页面、技能、MCP…")}
            className="w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
        </div>
        <div className="max-h-[50vh] overflow-y-auto p-2">
          {filtered.length === 0 ? (
            <p className="px-3 py-6 text-center text-sm text-muted-foreground">
              {t("没有匹配的页面或资源。")}
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
        <div className="border-t px-4 py-2 text-[11px] text-muted-foreground">{t("↑↓ 选择 · Enter 执行 · Esc 关闭")}</div>
      </div>
    </div>
  );
}
