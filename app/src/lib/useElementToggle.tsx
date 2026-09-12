import { toast } from "sonner";
import type { PolicyData } from "./contract";
import { runEngine } from "./engine";
import { POLICY_LAYER_COPY, TOGGLE_COPY } from "./ui-copy";
import { useExecuteApply, usePlanPreview } from "./usePlanPreview";

/**
 * 元素启停开关编排（Agents/MCP/Skills 集合等域共用）：
 * policy.set <enabledKey> <bool>（固定写入 local）→ toast 给「立即同步」动作
 * → 点开即 plan 预览（含 install-remove 二次确认语义）→ 域级 apply。
 */
export function useElementToggle() {
  const preview = usePlanPreview();
  const executeApply = useExecuteApply();

  const toggle = async (input: {
    /** 形如 agents:claude-code 的元素 id（用于 plan 过滤与提示）。 */
    elementId: string;
    /** 启停键，如 agents.claude-code.enabled。 */
    enabledKey: string;
    /** 当前启用态（引擎 policy.enabled）。 */
    enabled: boolean;
    /** 域，如 agents / mcp / skills。 */
    domain: string;
    /** 人类可读名称，如 Claude Code。 */
    name: string;
  }) => {
    const next = !input.enabled;
    let envelope;
    try {
      envelope = await runEngine<PolicyData>("policy:set", [
        "--layer",
        "local",
        "--key",
        input.enabledKey,
        "--value",
        next ? "true" : "false",
      ]);
    } catch (err) {
      toast.error(`${TOGGLE_COPY.saveFailed}:${err instanceof Error ? err.message : String(err)}`);
      return;
    }
    if (!envelope.ok) {
      toast.error(`${TOGGLE_COPY.saveRejected}:${envelope.errors[0]?.message ?? "未知错误"}`);
      return;
    }
    const label = `${next ? "启用" : "停用"} ${input.name}`;
    const applyLabel = `同步 ${input.name}`;
    toast.success(TOGGLE_COPY.savedAskApply(next ? "启用" : "停用", POLICY_LAYER_COPY.sqlite.label), {
      action: {
        label: TOGGLE_COPY.applyNow,
        onClick: () =>
          preview.open({
            title: label,
            subtitle: "仅同步此组件配置；目标被外部修改时会拒绝覆盖。",
            planCommands: [{ command: `${input.domain}:plan`, args: ["--only", input.elementId] }],
            requireConfirmText: input.domain === "agents" ? input.elementId : undefined,
            confirmLabel: TOGGLE_COPY.applyNow,
            execute: executeApply({
              command: `${input.domain}:apply`,
              label: applyLabel,
              extraArgs: ["--only", input.elementId],
            }),
          }),
      },
    });
  };

  return { toggle, modal: preview.modal };
}
