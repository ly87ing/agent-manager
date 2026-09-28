import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import type { PolicyData, PolicyLayers } from "./contract";
import { definitionContainerKey, definitionKeyPrefix, buildDefinitionRemoval, type DefinitionSection } from "./definitionOverlay";
import { engineErrorMessage, runEngine } from "./engine";
import { engineKeys } from "./queries";
import { OVERLAY_COPY, TOGGLE_COPY } from "./ui-copy";
import { toastWithUndo } from "./undo";
import { useDirectApply } from "./useDirectApply";

/**
 * 定义覆盖层的新增、编辑与移除；Agent 移除仅停止管理，
 * Skills 移除后直接 scoped apply 清理目标链接。
 */
export function useDefinitionOps({
  section,
  domain,
}: {
  section: DefinitionSection;
  /** apply 命令按生效域推导。 */
  domain: DefinitionSection;
}) {
  const queryClient = useQueryClient();
  const directApply = useDirectApply();
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

  /** 编辑且缺完整原定义（无 DB 的旧存储）：先 db:migrate 初始化（迁移本身会创建数据库），
   * 再只读 policy:get 取 sqlite 有效定义；失败或仍取不到则停止保存，定义不写入。
   * 取消表单不会走到这里，绝不迁移。 */
  const migrateAndLoadFullDefinition = async (name: string): Promise<Record<string, unknown> | null> => {
    const fail = (message: string) => {
      toast.error(`保存定义 ${name} 失败:${message}`);
      return null;
    };
    let migrated;
    try {
      migrated = await runEngine("db:migrate", []);
    } catch (err) {
      return fail(`初始化本地数据库失败:${engineErrorMessage(err)}`);
    }
    if (!migrated.ok) {
      return fail(`初始化本地数据库被拒绝:${migrated.errors[0]?.message ?? "未知错误"}`);
    }
    let policy;
    try {
      policy = await runEngine<PolicyData>("policy:get", []);
    } catch (err) {
      return fail(`读取完整定义失败:${engineErrorMessage(err)}`);
    }
    if (!policy.ok) {
      return fail(`读取完整定义被拒绝:${policy.errors[0]?.message ?? "未知错误"}`);
    }
    const sqlite = policy.data.layers?.sqlite as Record<string, unknown> | null | undefined;
    const sectionData = sqlite?.[section] as Record<string, unknown> | undefined;
    const defs = sectionData?.[definitionContainerKey(section)] as Record<string, unknown> | undefined;
    const full = defs?.[name];
    if (typeof full !== "object" || full === null) {
      return fail("初始化后仍取不到完整定义，已停止保存（定义未改动）");
    }
    return full as Record<string, unknown>;
  };

  /** 新增/编辑：写定义。Agent 与资源定义（技能集合）只保存定义，
   * 不写启用键、不 apply/preflight、不触发安装/同步。
   * 资源定义始终整条替换（_replace）：编辑时若调用方拿不到完整原定义（无 DB 的旧存储），
   * 先 db:migrate 再取 sqlite 有效定义合并表单字段，避免丢掉未展示字段或复活已清空字段。 */
  const saveDefinition = async (input: {
    name: string;
    definition: Record<string, unknown>;
    /** true = 新增；false = 编辑（不动启用键）。 */
    isNew: boolean;
    /** definition 是否为完整整条定义（skills 用）。新增恒为完整；编辑时调用方有
     * sqlite 有效定义则传完整定义（complete 可缺省），拿不到完整原定义必须显式传 false。 */
    complete?: boolean;
    /** Agent 定义表单打开时看到的内容；写入事务内据此拒绝过期保存。 */
    expectedDefinition?: Record<string, unknown> | null;
    /** 成功保存返回 true；被校验或引擎拒绝返回 false，调用方据此决定要不要接着落地。 */
  }): Promise<boolean> => {
    const defKey = `${definitionKeyPrefix(section)}.${input.name}`;
    let definition = input.definition;
    if (domain !== "agents" && !input.isNew && input.complete === false) {
      const full = await migrateAndLoadFullDefinition(input.name);
      if (!full) return false;
      definition = { ...full, ...definition };
    }
    const args = ["--layer", "local", "--key", defKey, "--value", JSON.stringify({ ...definition, _delete: false, _replace: true })];
    if (input.expectedDefinition !== undefined) {
      args.push("--expect-definition", JSON.stringify(input.expectedDefinition));
    }
    if (!(await writePolicy("policy:set", args))) {
      return false;
    }
    // 表单提交语义为「保存定义」：只保存定义，不写启用键、不 apply/preflight、不触发安装或
    // 同步；_replace 整条替换保证清空的可选字段不会复活。
    void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    toast.success(
      domain === "agents"
        ? `已保存定义 ${input.name}（不会自动安装）`
        : `已保存定义 ${input.name} 到本地资源库（不会自动安装或同步）`,
    );
    return true;
  };

  /** 被移除定义的完整内容：撤销时原样写回；取不到就不提供撤销（不假装能复原）。 */
  const fullDefinitionOf = (layers: PolicyLayers | null, name: string): Record<string, unknown> | null => {
    const sqlite = layers?.sqlite as Record<string, unknown> | null | undefined;
    const root = sqlite?.[section] as Record<string, unknown> | undefined;
    const defs = root?.[definitionContainerKey(section)] as Record<string, unknown> | undefined;
    const definition = defs?.[name];
    return typeof definition === "object" && definition !== null ? (definition as Record<string, unknown>) : null;
  };

  /** 移除确认后写遮蔽标记。 */
  const requestRemove = (name: string, layers: PolicyLayers | null) => {
    setRemoval({ name, layers });
  };

  const confirmRemove = async () => {
    if (!removal) return;
    if (domain === "skills") {
      const name = removal.name;
      setRemoval(null);
      // 直接 scoped apply 清理目标配置与链接
      void directApply.apply({
        command: "skills:apply",
        label: `移除 ${name}`,
        args: ["--only", `skill:${name}`, "--action", "remove"],
      });
      return;
    }
    setRemovalExecuting(true);
    const removalSpec = buildDefinitionRemoval(removal.layers, section, removal.name);
    const ok = await writePolicy("policy:set", ["--layer", removalSpec.layer, "--key", removalSpec.key, "--value", '{"_delete": true}']);
    setRemovalExecuting(false);
    if (!ok) return;
    const name = removal.name;
    const snapshot = fullDefinitionOf(removal.layers, name);
    setRemoval(null);
    void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    if (snapshot) {
      toastWithUndo(`已移除 ${name}`, {
        description:
          domain === "agents"
            ? "撤销会把定义原样加回来。"
            : "撤销只加回定义；已经清理掉的目标配置要再点一次应用才会写回。",
        onUndo: async () => {
          const restored = await writePolicy("policy:set", [
            "--layer",
            "local",
            "--key",
            `${definitionKeyPrefix(section)}.${name}`,
            "--value",
            JSON.stringify({ ...snapshot, _delete: false, _replace: true }),
          ]);
          if (restored) {
            void queryClient.invalidateQueries({ queryKey: engineKeys.all });
            toast.success(`已恢复 ${name}`);
          }
        },
      });
    } else {
      toast.success(`已移除 ${name}`);
    }
  };

  const removalSpec = removal ? buildDefinitionRemoval(removal.layers, section, removal.name) : null;
  const removalModal = removal && removalSpec ? (
    <ConfirmModal
      title={`移除 ${removal.name}`}
      description={domain === "agents" ? "停止管理该定义，不自动卸载系统包。后续同步仍会清理本工具拥有的孤立缓存目录。" : OVERLAY_COPY.removeDefinitionNote}
      risk="install-remove"
      requireText={removal.name}
      confirmLabel="确认移除"
      executing={removalExecuting}
      resultErrors={null}
      onConfirm={() => void confirmRemove()}
      onClose={() => setRemoval(null)}
    />
  ) : null;

  return { saveDefinition, requestRemove, modals: removalModal };
}
