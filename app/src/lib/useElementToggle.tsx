import { useRef, useState } from "react";
import { toast } from "sonner";
import type { PolicyData } from "./contract";
import { runEngine } from "./engine";
import { TOGGLE_COPY } from "./ui-copy";
import { useDirectApply } from "./useDirectApply";

/**
 * 元素启停开关编排（Agents/MCP/Skills 集合等域共用）：
 * policy:set <enabledKey> <bool>（固定写入 local）成功后立即精确 scoped apply；
 * 保存失败或被拒绝时不 apply，避免状态不一致。
 * 同步 ref 门闩保证同一 tick 重复点击零 policy:set、零 apply。
 */
export function useElementToggle() {
  const directApply = useDirectApply();
  const busyRef = useRef(false);
  const [executing, setExecuting] = useState(false);

  const toggle = async (input: {
    /** 形如 agents:claude-code 的元素 id（用于精确同步范围与提示）。 */
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
    if (busyRef.current) return;
    busyRef.current = true;
    setExecuting(true);

    const next = !input.enabled;
    try {
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
      await directApply.apply({
        command: `${input.domain}:apply`,
        label: `同步 ${input.name}`,
        args: ["--only", input.elementId],
      });
    } finally {
      busyRef.current = false;
      setExecuting(false);
    }
  };

  return { toggle, executing };
}
