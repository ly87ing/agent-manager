import { FolderOpen } from "lucide-react";
import { useAccountGroups } from "../lib/useAccountNames";
import { AgentIcon } from "./AgentIcon";
import type { SkillInstalledEntry } from "../lib/contract";
import { cn } from "../lib/utils";

/**
 * 当前安装：各助手技能目录里实际有什么（引擎 skills:status 的 installed）。
 * 只展示、不给操作——外部链接与真实目录不是待处理项；要纳入管理去来源页「未纳管」。
 */

const ORIGIN_VIEW: Record<string, { label: string; className: string }> = {
  managed: { label: "本工具链接", className: "border-primary/30 bg-primary/10 text-primary" },
  "external-link": { label: "外部链接", className: "border-info/30 bg-info/10 text-info" },
  "collection-extra-link": { label: "外部链接", className: "border-info/30 bg-info/10 text-info" },
  "native-dir": { label: "真实目录", className: "border bg-muted text-muted-foreground" },
  "legacy-link": { label: "旧版遗留链接", className: "border-warning/30 bg-warning/10 text-warning" },
  "dead-link": { label: "失效链接", className: "border-warning/30 bg-warning/10 text-warning" },
};

/** 摘要里的三大类，其余（失效、旧版遗留）有才显示。 */
const SUMMARY_ORDER: { key: string; label: string; origins: string[] }[] = [
  { key: "managed", label: "本工具链接", origins: ["managed"] },
  { key: "external", label: "外部链接", origins: ["external-link", "collection-extra-link"] },
  { key: "native", label: "真实目录", origins: ["native-dir"] },
  { key: "legacy", label: "旧版遗留链接", origins: ["legacy-link"] },
  { key: "dead", label: "失效链接", origins: ["dead-link"] },
];

function detailOf(entry: SkillInstalledEntry): string | null {
  if (entry.origin === "managed") return entry.collection ? `来自集合 ${entry.collection}` : null;
  if (entry.origin === "native-dir") return null;
  if (entry.origin === "collection-extra-link" && entry.collection) {
    return `→ ${entry.target ?? "?"}（与集合 ${entry.collection} 同名）`;
  }
  return entry.target ? `→ ${entry.target}` : null;
}

export function SkillInstalledView({ entries, agents }: { entries: SkillInstalledEntry[]; agents: string[] }) {
  const groupsOf = useAccountGroups();
  // 按账号页的顺序：同一助手的账号挨在一起，主号在前
  const columns = groupsOf(Array.from(new Set([...agents, ...entries.map((entry) => entry.agent)])).sort())
    .flatMap((group) => group.columns.map((column) => ({ ...column, kind: group.kind })));
  return (
    <section data-testid="skills-installed" className="rounded-lg border bg-card p-4 shadow-sm">
      <h3 className="flex items-center gap-2 text-sm font-semibold text-foreground">
        <FolderOpen className="h-4 w-4 text-primary" aria-hidden />
        当前安装
      </h3>
      <p className="mt-1 text-xs text-muted-foreground">
        各账号技能目录里现在实际有什么。这里只展示：外部链接和真实目录由它们原来的方式维护，不需要处理。
      </p>
      {entries.length === 0 ? (
        <p data-testid="skills-installed-empty" className="mt-3 text-xs text-muted-foreground">
          各账号的技能目录里还没有技能。
        </p>
      ) : (
        <div className="mt-3 space-y-2">
          {columns.map(({ id: agent, fullName, kind }) => {
            const own = entries.filter((entry) => entry.agent === agent);
            const summary = SUMMARY_ORDER.map(({ key, label, origins }) => ({
              key,
              label,
              count: own.filter((entry) => origins.includes(entry.origin)).length,
            })).filter(({ key, count }) => count > 0 || ["managed", "external", "native"].includes(key));
            return (
              <details key={agent} data-testid={`skills-installed-${agent}`} className="rounded-md border px-3 py-2">
                <summary className="cursor-pointer text-xs text-foreground">
                  <span className="inline-flex items-center gap-1.5 align-middle font-semibold">
                    <AgentIcon kind={kind} />
                    {fullName}
                  </span>
                  <span className="ml-2 text-muted-foreground">
                    {own.length} 项：
                    {summary.map(({ key, label, count }, index) => (
                      <span key={key} data-testid={`skills-installed-${agent}-${key}`}>
                        {index > 0 ? " · " : ""}
                        {label} {count}
                      </span>
                    ))}
                  </span>
                </summary>
                {own.length === 0 ? (
                  <p className="mt-2 text-xs text-muted-foreground">这个账号的技能目录里还没有技能。</p>
                ) : (
                  <ul className="mt-2 space-y-1">
                    {own.map((entry) => {
                      const view = ORIGIN_VIEW[entry.origin] ?? { label: entry.origin, className: "border bg-muted text-muted-foreground" };
                      const detail = detailOf(entry);
                      return (
                        <li
                          key={entry.name}
                          data-testid={`skills-installed-entry-${agent}-${entry.name}`}
                          data-origin={entry.origin}
                          className="flex flex-wrap items-center gap-1.5 text-xs"
                          title={entry.link_path}
                        >
                          <span className="font-mono text-foreground">{entry.name}</span>
                          <span className={cn("rounded border px-1 py-0.5 text-[10px]", view.className)}>{view.label}</span>
                          {detail && <span className="break-all font-mono text-[11px] text-muted-foreground">{detail}</span>}
                        </li>
                      );
                    })}
                  </ul>
                )}
              </details>
            );
          })}
        </div>
      )}
    </section>
  );
}
