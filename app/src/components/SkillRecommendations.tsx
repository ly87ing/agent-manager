import { Plus, Sparkles } from "lucide-react";
import type { SkillRecommendation } from "../lib/contract";

/**
 * 推荐来源：编译内置、只读（引擎 skills:status 的 recommendations）。
 * 不是集合、不计入待同步；只有用户点「添加」并确认后，才把它保存为一条普通集合定义。
 */
export function SkillRecommendations({
  items,
  onAdd,
}: {
  items: SkillRecommendation[];
  onAdd: (item: SkillRecommendation) => void;
}) {
  return (
    <section data-testid="skills-recommendations" className="rounded-lg border border-dashed bg-card p-4">
      <h3 className="flex items-center gap-2 text-sm font-semibold text-foreground">
        <Sparkles className="h-4 w-4 text-primary" aria-hidden />
        推荐来源
      </h3>
      <p className="mt-1 text-xs text-muted-foreground">
        内置推荐，添加后才会保存为集合。添加不会下载，也不会启用到任何助手；之后按需逐个启用。
      </p>
      <div className="mt-3 space-y-2">
        {items.map((item) => (
          <div
            key={item.name}
            data-testid={`recommendation-${item.name}`}
            className="flex flex-wrap items-start justify-between gap-3 rounded-md border px-3 py-2"
          >
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="font-mono text-xs font-semibold text-foreground">{item.name}</span>
                <span className="rounded border bg-muted px-1 py-0.5 text-[10px] text-muted-foreground">
                  {item.skill_paths.length} 个技能
                </span>
              </div>
              <p className="mt-0.5 break-all font-mono text-[11px] text-muted-foreground">{item.url}</p>
              <p className="mt-1 text-xs text-muted-foreground">{item.description}</p>
            </div>
            {item.added ? (
              <span
                data-testid={`recommendation-added-${item.name}`}
                className="rounded border border-success/30 bg-success/10 px-2 py-0.5 text-xs text-success"
              >
                已添加
              </span>
            ) : (
              <button
                type="button"
                data-testid={`recommendation-add-${item.name}`}
                onClick={() => onAdd(item)}
                className="inline-flex items-center gap-1 rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15"
              >
                <Plus className="h-3.5 w-3.5" aria-hidden /> 添加
              </button>
            )}
          </div>
        ))}
      </div>
    </section>
  );
}
