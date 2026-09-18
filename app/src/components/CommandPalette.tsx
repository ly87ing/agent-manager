import { useEffect, useMemo, useRef, useState } from "react";
import { Search } from "lucide-react";
import type { StatusData } from "../lib/contract";
import { useEngineQuery } from "../lib/queries";
import { useDirectApply } from "../lib/useDirectApply";
import { useRefreshUpstream } from "../lib/queries";
import { matchesKeyword } from "./SearchInput";
import { cn } from "../lib/utils";

export interface CommandItem {
  id: string;
  label: string;
  /** 右侧说明：分类、所属助手或目标路径。 */
  hint?: string;
  /** 额外匹配词（英文名、命令名等）。 */
  keywords?: string;
  group: string;
  run: () => void;
}

const PAGES: { key: string; label: string; keywords: string }[] = [
  { key: "dashboard", label: "总览", keywords: "dashboard 状态 诊断" },
  { key: "agents", label: "助手", keywords: "agents claude codex 实例" },
  { key: "skills", label: "技能", keywords: "skills 技能矩阵" },
  { key: "mcp", label: "MCP", keywords: "mcp 服务器" },
  { key: "rules-hooks", label: "规则与 Hooks", keywords: "rules hooks 规则" },
  { key: "takeover", label: "接管中心", keywords: "takeover 接管 发现" },
  { key: "runs", label: "操作记录", keywords: "runs 任务 日志" },
  { key: "settings", label: "设置", keywords: "settings 偏好 终端快捷命令" },
  // 设置里的二级页面也要搜得到：密钥现在是要填东西的页，藏在二级菜单里不好找
  { key: "settings:secrets", label: "密钥", keywords: "secrets 密钥 变量 token api key 凭据" },
  { key: "settings:aliases", label: "终端快捷命令", keywords: "aliases alias 别名 启动命令 时区" },
];

/**
 * 命令面板（⌘K）：一个入口搜到所有资源、页面与常用动作，键盘直接完成。
 *
 * 资源只从已加载的查询缓存里取（面板打开时才发起首次查询），点中资源就跳到它所在的页面；
 * 动作走与页面上相同的执行通道，因此锁、审计与结果提示都一致。
 */
export function CommandPalette({
  open,
  onClose,
  onNavigate,
  onOpenAgentSkills,
}: {
  open: boolean;
  onClose: () => void;
  onNavigate: (page: string) => void;
  /** 选中某个助手/实例的技能时，跳到技能页并预选它。 */
  onOpenAgentSkills: (agentId: string) => void;
}) {
  const [keyword, setKeyword] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const directApply = useDirectApply();
  const refreshUpstream = useRefreshUpstream();

  const agents = useEngineQuery<StatusData>("agents:status", [], { enabled: open });
  const skills = useEngineQuery<StatusData>("skills:status", [], { enabled: open });
  const mcp = useEngineQuery<StatusData>("mcp:status", [], { enabled: open });
  const profiles = useEngineQuery<StatusData>("profiles:status", [], { enabled: open });

  useEffect(() => {
    if (open) {
      setKeyword("");
      setActive(0);
      // 面板一打开就能直接敲字
      window.setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [open]);

  const items = useMemo<CommandItem[]>(() => {
    const out: CommandItem[] = PAGES.map((page) => ({
      id: `page:${page.key}`,
      label: page.label,
      hint: "页面",
      keywords: page.keywords,
      group: "前往",
      run: () => onNavigate(page.key),
    }));

    for (const element of agents.data?.ok ? agents.data.data.elements : []) {
      const id = element.id.replace(/^agents:/, "");
      const spec = element.spec as { display_name?: string; bin?: string };
      out.push({
        id: `agent:${id}`,
        label: spec.display_name ?? id,
        hint: "助手",
        keywords: `${id} ${spec.bin ?? ""}`,
        group: "资源",
        run: () => onNavigate("agents"),
      });
      out.push({
        id: `agent-skills:${id}`,
        label: `${spec.display_name ?? id} 的技能`,
        hint: "技能分配",
        keywords: `${id} skills`,
        group: "资源",
        run: () => onOpenAgentSkills(id),
      });
    }
    for (const element of profiles.data?.ok ? profiles.data.data.elements : []) {
      const id = element.id.replace(/^profiles:/, "");
      const spec = element.spec as { display_name?: string; config_dir?: string };
      out.push({
        id: `profile:${id}`,
        label: spec.display_name ?? id,
        hint: `实例 · ${spec.config_dir ?? ""}`,
        keywords: `${id} profile 实例`,
        group: "资源",
        run: () => onNavigate("agents"),
      });
    }
    for (const element of skills.data?.ok ? skills.data.data.elements : []) {
      const id = element.id.replace(/^skill:/, "");
      if (id.includes("/")) continue;
      out.push({
        id: `skill:${id}`,
        label: id,
        hint: "技能集合",
        keywords: `${id} skills`,
        group: "资源",
        run: () => onNavigate("skills"),
      });
    }
    for (const element of mcp.data?.ok ? mcp.data.data.elements : []) {
      const id = element.id.replace(/^mcp:/, "");
      out.push({
        id: `mcp:${id}`,
        label: id,
        hint: "MCP 服务",
        keywords: `${id} mcp`,
        group: "资源",
        run: () => onNavigate("mcp"),
      });
    }

    out.push(
      {
        id: "action:refresh-upstream",
        label: "检查更新",
        hint: "对比官方最新版本",
        keywords: "update upstream 版本",
        group: "动作",
        run: () => refreshUpstream.mutate(undefined),
      },
      {
        id: "action:apply-rules",
        label: "同步规则",
        hint: "写入 AGENTS.md 与 CLAUDE.md",
        keywords: "rules 规则 同步",
        group: "动作",
        run: () => void directApply.apply({ command: "rules:apply", label: "同步规则" }),
      },
      {
        id: "action:apply-aliases",
        label: "同步终端快捷命令",
        hint: "写入 .zshrc 与 .zprofile",
        keywords: "alias 别名 启动命令",
        group: "动作",
        run: () => void directApply.apply({ command: "aliases:apply", label: "同步终端快捷命令" }),
      },
      {
        id: "action:takeover",
        label: "扫描本机资源",
        hint: "打开接管中心",
        keywords: "discover 接管 扫描",
        group: "动作",
        run: () => onNavigate("takeover"),
      },
    );
    return out;
  }, [agents.data, skills.data, mcp.data, profiles.data, directApply, refreshUpstream, onNavigate, onOpenAgentSkills]);

  const filtered = useMemo(
    () => items.filter((item) => matchesKeyword(keyword, [item.label, item.hint, item.keywords])),
    [items, keyword],
  );

  useEffect(() => {
    setActive((prev) => (prev >= filtered.length ? 0 : prev));
  }, [filtered.length]);

  if (!open) return null;

  const runItem = (item: CommandItem) => {
    item.run();
    onClose();
  };

  const groups = filtered.reduce<Record<string, CommandItem[]>>((acc, item) => {
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
        aria-label="命令面板"
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
            onChange={(event) => setKeyword(event.target.value)}
            data-testid="command-palette-input"
            aria-label="搜索资源、页面与动作"
            placeholder="搜索助手、技能、MCP、实例，或直接执行常用动作…"
            className="w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
        </div>
        <div className="max-h-[50vh] overflow-y-auto p-2">
          {filtered.length === 0 ? (
            <p className="px-3 py-6 text-center text-sm text-muted-foreground">没有匹配的资源或动作。</p>
          ) : (
            Object.entries(groups).map(([group, groupItems]) => (
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
        <div className="border-t px-4 py-2 text-[11px] text-muted-foreground">
          ↑↓ 选择 · Enter 执行 · Esc 关闭
        </div>
      </div>
    </div>
  );
}
