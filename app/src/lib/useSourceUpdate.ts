import { t } from "./i18n";
import { useRef, useState } from "react";
import { toast } from "./notifications";
import { useQueryClient } from "@tanstack/react-query";
import type { EngineElement, StatusData } from "./contract";
import { engineErrorMessage, runEngine } from "./engine";
import { batchCounts } from "./batchResult";
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
  const busyRef = useRef(false);

  const update = async (targets: EngineElement[]): Promise<void> => {
    if (busyRef.current || targets.length === 0) return;
    busyRef.current = true;
    setUpdating(true);
    let fetched = 0;
    let failed = 0;
    const ids = new Set<string>();
    try {
      for (const collection of targets) {
        const envelope = await execute({
          command: "skills:apply",
          label: t("更新来源 {0}", collection.id.replace(/^skill:/, "")),
          args: ["--only", collection.id, "--action", "update"],
          silent: true,
        });
        if (envelope?.ok) {
          fetched += 1;
          ids.add(collection.id);
          await runEngine("system:refresh-upstream", ["--only", collection.id]).catch(() => undefined);
        } else {
          failed += 1;
        }
      }
      // 取回之后按最新状态算哪些技能有新版本，只换这些来源的
      const fresh = await runEngine<StatusData>("skills:status", []);
      const sourceSummary = t("来源更新：成功 {0}，失败 {1}", fetched, failed);
      if (!fresh.ok) {
        toast[fetched ? "warning" : "error"](sourceSummary, { description: t("无法读取更新后的技能状态，尚未确认部署结果：{0}", fresh.errors[0]?.message ?? t("读取失败")) });
        return;
      }
      const rows = versionUpdateRows(fresh.data.elements ?? []).filter((row) =>
        ids.has(row.elementId.slice(0, row.elementId.lastIndexOf("/"))),
      );
      if (rows.length === 0) {
        toast[failed ? fetched ? "warning" : "error" : "success"](sourceSummary, { description: t("已成功更新的来源没有待部署的技能版本") });
        return;
      }
      const result = await batchApply.applyBatch({ actionLabel: t("更新"), rows, silent: true });
      if (!result) {
        toast.warning(sourceSummary, { description: t("技能部署尚未执行，请等待当前任务结束后重试") });
        return;
      }
      const counts = batchCounts(result);
      toast[failed || counts.failed ? fetched || counts.succeeded ? "warning" : "error" : "success"](sourceSummary, { description: t("技能部署：成功 {0}，失败 {1}{2}", counts.succeeded, counts.failed, failed || counts.failed ? t("（详见任务记录）") : "") });
    } catch (error) {
      toast[fetched ? "warning" : "error"](t("来源更新：成功 {0}，失败 {1}", fetched, failed), { description: engineErrorMessage(error) });
    } finally {
      try { await queryClient.invalidateQueries({ queryKey: engineKeys.all }); }
      finally { busyRef.current = false; setUpdating(false); }
    }
  };

  return { update, updating: updating || batchApply.executing };
}
