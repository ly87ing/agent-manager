import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import type { PolicyData, PolicyLayers } from "./contract";
import { definitionKeyPrefix, planDefinitionRemoval, type DefinitionSection } from "./definitionOverlay";
import { engineErrorMessage, runEngine } from "./engine";
import { getDefaultPolicyLayer } from "./policyLayer";
import { engineKeys } from "./queries";
import { OVERLAY_COPY, TOGGLE_COPY } from "./ui-copy";
import { useExecuteApply, usePlanPreview } from "./usePlanPreview";

/**
 * 定义覆盖层编排（MCP 服务 / Skills 集合的新增·编辑·移除）。
 * 新增 = policy.set <覆盖键> <定义> + policy.set <启用键> true；编辑基线条目 = 同键写覆盖片段；
 * 移除 = 覆盖层条目 policy:delete / 基线条目 tombstone（{"_delete": true}）。
 * 全部写操作后接 plan 预览 → 域级 apply（引擎单写者）。
 */
export function useDefinitionOps({
  section,
  domain,
}: {
  section: DefinitionSection;
  /** 生效域（mcp / skills），plan/apply 命令按它推导。 */
  domain: "mcp" | "skills";
}) {
  const queryClient = useQueryClient();
  const preview = usePlanPreview();
  const executeApply = useExecuteApply();
  const [removal, setRemoval] = useState<{ name: string; layers: PolicyLayers | null } | null>(null);
  const [removalExecuting, setRemovalExecuting] = useState(false);

  /** policy.set / policy:delete 的公共调用；失败 toast 并返回 false。 */
  const writePolicy = async (command: "policy:set" | "policy:delete", args: string[]): Promise<boolean> => {
    let envelope;
    try {
      envelope = await runEngine<PolicyData>(command, args);
    } catch (err) {
      toast.error(`${TOGGLE_COPY.saveFailed}:${engineErrorMessage(err)}`);
      return false;
    }
    if (!envelope.ok) {
      toast.error(`${TOGGLE_COPY.saveRejected}:${envelope.errors[0]?.message ?? "未知错误"}`);
      return false;
    }
    return true;
  };

  /** 新增/编辑：写定义片段（新增时同时写启用键）→ plan 预览 → 域级 apply。 */
  const saveAndApply = async (input: {
    name: string;
    definition: Record<string, unknown>;
    /** true = 新增（同时写 enabled=true）；false = 编辑（不动启用键）。 */
    isNew: boolean;
    /** 预览过滤的元素 id（mcp:<name> / skill:<name>）。 */
    elementId: string;
  }): Promise<void> => {
    const layer = getDefaultPolicyLayer();
    const defKey = `${definitionKeyPrefix(section)}.${input.name}`;
    if (!(await writePolicy("policy:set", ["--layer", layer, "--key", defKey, "--value", JSON.stringify(input.definition)]))) {
      return;
    }
    if (input.isNew) {
      if (!(await writePolicy("policy:set", ["--layer", layer, "--key", `${section}.${input.name}.enabled`, "--value", "true"]))) {
        return;
      }
    }
    void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    const label = `${input.isNew ? "新增" : "修改"} ${input.name}`;
    preview.open({
      title: label,
      subtitle: "定义已写入覆盖层；预览将要执行的操作，确认后同步生效（执行前自动备份）。",
      planCommands: [`${domain}:plan`],
      filterElement: input.elementId,
      confirmLabel: TOGGLE_COPY.applyNow,
      execute: executeApply({ command: `${domain}:apply`, label }),
    });
  };

  /** 移除：弹确认（install-remove 语义，输入条目名）；确认后 delete/tombstone → plan 预览。 */
  const requestRemove = (name: string, layers: PolicyLayers | null) => {
    setRemoval({ name, layers });
  };

  const confirmRemove = async () => {
    if (!removal) return;
    setRemovalExecuting(true);
    const plan = planDefinitionRemoval(removal.layers, section, removal.name, getDefaultPolicyLayer());
    const ok =
      plan.kind === "overlay"
        ? await writePolicy("policy:delete", ["--layer", plan.layer, "--key", plan.key])
        : await writePolicy("policy:set", ["--layer", plan.layer, "--key", plan.key, "--value", '{"_delete": true}']);
    setRemovalExecuting(false);
    if (!ok) return;
    const name = removal.name;
    setRemoval(null);
    void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    toast.success(`已移除 ${name}`);
    preview.open({
      title: `移除 ${name}`,
      subtitle: plan.kind === "tombstone" ? OVERLAY_COPY.removeBaselineNote : "移除已写入覆盖层；预览将要执行的操作，确认后同步生效。",
      planCommands: [`${domain}:plan`],
      confirmLabel: TOGGLE_COPY.applyNow,
      execute: executeApply({ command: `${domain}:apply`, label: `移除 ${name}` }),
    });
  };

  const removalPlan = removal ? planDefinitionRemoval(removal.layers, section, removal.name, getDefaultPolicyLayer()) : null;
  const removalModal = removal && removalPlan ? (
    <ConfirmModal
      title={`移除 ${removal.name}`}
      description={removalPlan.kind === "tombstone" ? OVERLAY_COPY.removeBaselineNote : OVERLAY_COPY.removeOverlayNote}
      risk="install-remove"
      requireText={removal.name}
      confirmLabel="确认移除"
      executing={removalExecuting}
      resultErrors={null}
      onConfirm={() => void confirmRemove()}
      onClose={() => setRemoval(null)}
    />
  ) : null;

  return { saveAndApply, requestRemove, modals: (
    <>
      {removalModal}
      {preview.modal}
    </>
  ) };
}
