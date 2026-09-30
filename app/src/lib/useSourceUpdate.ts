import { useState } from "react";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import type { EngineElement, StatusData } from "./contract";
import { runEngine } from "./engine";
import { useExecutions } from "./execution";
import type { SkillCollectionObserved } from "./observed";
import { engineKeys } from "./queries";
import { versionUpdateRows } from "./skillFill";
import { useBatchApply } from "./useBatchApply";

/** 能取回新内容的来源：启用中、引擎给了 update。 */
export function updatableSources(collections: EngineElement[]): EngineElement[] {
  return collections.filter(
    (collection) =>
      (collection.policy as { enabled?: boolean }).enabled !== false &&
      ((collection.observed as SkillCollectionObserved).actions ?? []).includes("update"),
  );
}

/**
 * 更新来源：逐个取回（git 按跟踪方式拉到最新、本机目录重新扫描），再把各账号在用、已有新版本的
 * 技能换到新版本。新发现的技能不自动添加，账号上停用的不动；单个来源失败不拦其余。
 * 来源页的「全部更新」与待处理页的「更新」共用这一个。
 */
export function useSourceUpdate() {
  const { execute } = useExecutions();
  const batchApply = useBatchApply();
  const queryClient = useQueryClient();
  const [updating, setUpdating] = useState(false);

  const update = async (targets: EngineElement[]): Promise<void> => {
    if (updating || targets.length === 0) return;
    setUpdating(true);
    let fetched = 0;
    let failed = 0;
    try {
      for (const collection of targets) {
        const envelope = await execute({
          command: "skills:apply",
          label: `更新来源 ${collection.id.replace(/^skill:/, "")}`,
          args: ["--only", collection.id, "--action", "update"],
          silent: true,
        });
        if (envelope?.ok) {
          fetched += 1;
          await runEngine("system:refresh-upstream", ["--only", collection.id]).catch(() => undefined);
        } else {
          failed += 1;
        }
      }
      // 取回之后按最新状态算哪些技能有新版本，只换这些来源的
      const ids = new Set(targets.map((collection) => collection.id));
      const fresh = await runEngine<StatusData>("skills:status", []);
      const rows = (fresh.ok ? versionUpdateRows(fresh.data.elements ?? []) : []).filter((row) =>
        ids.has(row.elementId.slice(0, row.elementId.lastIndexOf("/"))),
      );
      (failed ? toast.error : toast.success)(
        `已取回 ${fetched} 个来源${failed ? `，${failed} 个失败（详见任务记录）` : ""}；${rows.length ? `${rows.length} 处技能换到新版本` : "各账号的技能都已是最新"}`,
      );
      await batchApply.applyBatch({ actionLabel: "更新", rows });
    } finally {
      setUpdating(false);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    }
  };

  return { update, updating: updating || batchApply.executing };
}
