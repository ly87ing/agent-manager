import { AccountCells, AccountHeader, RowActions } from "./AccountGrid";
import { matchesKeyword } from "./SearchInput";
import type { SkillUnmanaged, SkillUnmanagedView } from "../lib/contract";
import type { AccountGroup } from "../lib/accounts";

/** `https://github.com/owner/repo.git`、`git@host:owner/repo.git` → `owner/repo`。 */
export function shortRepository(url: string): string {
  const trimmed = url.replace(/\.git$/, "").replace(/\/+$/, "");
  const parts = trimmed.split(/[/:]/).filter(Boolean);
  return parts.length >= 2 ? `${parts[parts.length - 2]}/${parts[parts.length - 1]}` : trimmed;
}

/** 来源一行：仓库 · 版本 · 安装方式；认不出来时如实说。 */
export function sourceText(source: SkillUnmanaged["source"]): string {
  if (!source) return "来源不明";
  const parts: string[] = [];
  if (source.repository) parts.push(shortRepository(source.repository));
  if (source.version) parts.push(`v${source.version.replace(/^v/, "")}`);
  if (source.installer === "npx-skills") parts.push("npx skills 安装");
  return parts.join(" · ") || "来源不明";
}

const BLOCKED: Record<string, string> = {
  "claude-synced": "Claude 自己同步的",
  "no-skill-md": "没有 SKILL.md",
};

/**
 * 来源页的「未纳管」：各账号目录与 Codex 共用目录里、本工具没管理的技能，以及 Codex
 * 会加载两份的重名。每项可「纳入管理」或「清理多余」，具体做什么先在对话框里预览。
 */
export function SkillOtherSources({
  view,
  groups,
  search,
  agentFilter,
  onAdopt,
}: {
  view: SkillUnmanagedView;
  groups: AccountGroup[];
  search: string;
  agentFilter: string;
  /** 打开纳入 / 清理对话框（按技能名）。 */
  onAdopt: (name: string) => void;
}) {
  const sharedPath = (id: string) => view.shared_dirs.find((dir) => dir.id === id)?.path ?? id;
  const inAgent = (skill: SkillUnmanaged) =>
    agentFilter === "all" || skill.locations.some((location) => location.agent === agentFilter || location.shared !== null);
  const skills = view.skills.filter(
    (skill) => inAgent(skill) && matchesKeyword(search, [skill.name, skill.source?.repository ?? undefined]),
  );
  const duplicates = view.duplicates.filter(
    (dup) => (agentFilter === "all" || dup.agent === agentFilter) && matchesKeyword(search, [dup.name]),
  );
  if (skills.length === 0 && duplicates.length === 0) return null;

  return (
    <div data-testid="skills-other-sources" className="overflow-clip rounded-lg border bg-card">
      <AccountHeader
        groups={groups}
        left={
          <span className="font-medium text-foreground" title="用别的方式装进账号目录的技能：纳入管理后只留一份源，可撤回">
            未纳管 · {skills.length}
          </span>
        }
      />
      <ul className="divide-y">
        {skills.map((skill) => {
          const shared = skill.locations.filter((location) => location.shared !== null);
          const where = skill.locations.map((location) => location.path).join("\n");
          return (
            <li key={skill.name} data-testid={`other-skill-${skill.name}`} className="group flex items-center gap-3 py-1.5 pl-4 pr-4 hover:bg-accent/40">
              <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2" title={where}>
                <span className="min-w-0 truncate text-sm">{skill.name}</span>
                <span data-testid={`other-source-${skill.name}`} className="truncate text-xs text-muted-foreground" title={skill.source?.repository ?? undefined}>
                  {sourceText(skill.source)}
                </span>
                {shared.map((location) => (
                  <span
                    key={location.path}
                    data-testid={`other-shared-${skill.name}`}
                    className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground"
                    title={`${sharedPath(location.shared!)}：全部 Codex 账号都会用到，不能按账号关`}
                  >
                    Codex 共用
                  </span>
                ))}
                {skill.blocked_reason && (
                  <span data-testid={`other-blocked-${skill.name}`} className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground" title="只显示，不能纳入管理">
                    {BLOCKED[skill.blocked_reason] ?? skill.blocked_reason}
                  </span>
                )}
              </div>
              <AccountCells
                groups={groups}
                cell={(column) => {
                  const location = skill.locations.find((item) => item.agent === column.id);
                  return location ? (
                    <span
                      data-testid={`other-present-${skill.name}-${column.id}`}
                      className="inline-block h-2 w-2 rounded-full bg-muted-foreground/60"
                      title={`${column.fullName}：${location.path}${location.target ? ` → ${location.target}` : ""}`}
                    />
                  ) : null;
                }}
              />
              <RowActions
                always={
                  skill.adoptable ? (
                    <button
                      type="button"
                      data-testid={`adopt-${skill.name}`}
                      onClick={() => onAdopt(skill.name)}
                      className="rounded-md border px-2 py-0.5 text-xs hover:bg-accent"
                    >
                      纳入管理…
                    </button>
                  ) : null
                }
              />
            </li>
          );
        })}
        {duplicates.map((dup) => (
          <li key={`${dup.name}-${dup.agent}`} data-testid={`duplicate-${dup.name}-${dup.agent}`} className="group flex items-center gap-3 py-1.5 pl-4 pr-4 hover:bg-accent/40">
            <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2" title={dup.paths.join("\n")}>
              <span className="min-w-0 truncate text-sm">{dup.name}</span>
              <span className="rounded bg-warning/15 px-1.5 py-0.5 text-[11px] text-warning">Codex 会加载两份</span>
              <span className="truncate font-mono text-xs text-muted-foreground">{dup.paths[dup.paths.length - 1]}</span>
            </div>
            <AccountCells
              groups={groups}
              cell={(column) =>
                column.id === dup.agent ? (
                  <span className="inline-block h-2 w-2 rounded-full bg-warning" title={`${column.fullName}：${dup.paths.join(" 与 ")}`} />
                ) : null
              }
            />
            <RowActions
              always={
                <button
                  type="button"
                  data-testid={`dedupe-${dup.name}-${dup.agent}`}
                  onClick={() => onAdopt(dup.name)}
                  className="rounded-md border px-2 py-0.5 text-xs hover:bg-accent"
                >
                  清理多余…
                </button>
              }
            />
          </li>
        ))}
      </ul>
    </div>
  );
}
