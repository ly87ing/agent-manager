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
 * 定义覆盖层的新增、编辑与移除；Agent/Tools 移除仅停止管理，
 * MCP/Skills 移除后继续预览清理目标配置与链接。
 */
export function useDefinitionOps({
  section,
  domain,
}: {
  section: DefinitionSection;
  /** plan/apply 命令按生效域推导。 */
  domain: DefinitionSection;
}) {
  const queryClient = useQueryClient();
  const preview = usePlanPreview();
  const executeApply = useExecuteApply();
  const [removal, setRemoval] = useState<{ name: string; layers: PolicyLayers | null } | null>(null);
  const [removalExecuting, setRemovalExecuting] = useState(false);

  const writePolicy = async (command: "policy:set", args: string[]): Promise<boolean> => {
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
    if (!(await writePolicy("policy:set", ["--layer", layer, "--key", defKey, "--value", JSON.stringify({ ...input.definition, _delete: false, ...(domain === "agents" || domain === "tools" ? { _replace: true } : {}) })]))) {
      return;
    }
    // Agent 表单提交语义为“保存定义”：只保存定义，不写新启用键、不 plan/apply，不触发安装。
    if (domain === "agents") {
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
      toast.success(`已保存定义 ${input.name}（不会自动安装）`);
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
      planCommands: [{ command: `${domain}:plan`, args: ["--only", input.elementId] }],
      confirmLabel: TOGGLE_COPY.applyNow,
      execute: executeApply({
        command: `${domain}:apply`,
        label,
        extraArgs: ["--only", input.elementId],
      }),
    });
  };

  /** 移除确认后写遮蔽标记。 */
  const requestRemove = (name: string, layers: PolicyLayers | null) => {
    setRemoval({ name, layers });
  };

  const confirmRemove = async () => {
    if (!removal) return;
    setRemovalExecuting(true);
    const plan = planDefinitionRemoval(removal.layers, section, removal.name, getDefaultPolicyLayer());
    const ok = await writePolicy("policy:set", ["--layer", plan.layer, "--key", plan.key, "--value", '{"_delete": true}']);
    setRemovalExecuting(false);
    if (!ok) return;
    const name = removal.name;
    setRemoval(null);
    void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    toast.success(`已移除 ${name}`);
    if (domain === "agents" || domain === "tools") return;
    const prefix = domain === "skills" ? "skill" : domain;
    const elementId = `${prefix}:${name}`;
    preview.open({
      title: `移除 ${name}`,
      subtitle: OVERLAY_COPY.removeBaselineNote,
      planCommands: [{ command: `${domain}:plan`, args: ["--only", elementId] }],
      confirmLabel: TOGGLE_COPY.applyNow,
      execute: executeApply({
        command: `${domain}:apply`,
        label: `移除 ${name}`,
        extraArgs: ["--only", elementId],
      }),
    });
  };

  const removalPlan = removal ? planDefinitionRemoval(removal.layers, section, removal.name, getDefaultPolicyLayer()) : null;
  const removalModal = removal && removalPlan ? (
    <ConfirmModal
      title={`移除 ${removal.name}`}
      description={domain === "agents" || domain === "tools" ? "停止管理该定义，不自动卸载系统包。后续同步仍会清理本工具拥有的孤立缓存目录。" : OVERLAY_COPY.removeBaselineNote}
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
