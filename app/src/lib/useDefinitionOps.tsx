import { t } from "./i18n";
import { useQueryClient } from "@tanstack/react-query";
import { notifyError, toast } from "./notifications";
import type { PolicyData } from "./contract";
import { engineErrorMessage, runEngine } from "./engine";
import { engineKeys } from "./queries";

/** Save a skill source definition without installing or applying its resources. */
export function useDefinitionOps() {
  const queryClient = useQueryClient();

  const writePolicy = async (args: string[]): Promise<boolean> => {
    let envelope;
    try {
      envelope = await runEngine<PolicyData>("policy:set", args);
    } catch (err) {
      notifyError(t("保存来源失败"), err);
      return false;
    }
    if (!envelope.ok) {
      notifyError(t("保存来源失败"), envelope.errors[0]?.message ?? t("未知错误"));
      return false;
    }
    return true;
  };

  /** A legacy definition may need migration before its complete stored value can be edited. */
  const migrateAndLoadFullDefinition = async (name: string): Promise<Record<string, unknown> | null> => {
    const fail = (message: string) => {
      notifyError(t("保存定义 {0} 失败", name), message);
      return null;
    };
    let migrated;
    try {
      migrated = await runEngine("db:migrate", []);
    } catch (err) {
      return fail(t("初始化本地数据库失败:{0}", engineErrorMessage(err)));
    }
    if (!migrated.ok) {
      return fail(t("初始化本地数据库被拒绝:{0}", migrated.errors[0]?.message ?? t("未知错误")));
    }
    let policy;
    try {
      policy = await runEngine<PolicyData>("policy:get", []);
    } catch (err) {
      return fail(t("读取完整定义失败:{0}", engineErrorMessage(err)));
    }
    if (!policy.ok) {
      return fail(t("读取完整定义被拒绝:{0}", policy.errors[0]?.message ?? t("未知错误")));
    }
    const sqlite = policy.data.layers?.sqlite as Record<string, unknown> | null | undefined;
    const defs = (sqlite?.["skills"] as Record<string, unknown> | undefined)?.["repositories"] as Record<string, unknown> | undefined;
    const full = defs?.[name];
    if (typeof full !== "object" || full === null) {
      return fail(t("初始化后仍取不到完整定义，已停止保存（定义未改动）"));
    }
    return full as Record<string, unknown>;
  };

  const saveDefinition = async (input: {
    name: string;
    definition: Record<string, unknown>;
    isNew: boolean;
    complete?: boolean;
    silent?: boolean;
  }): Promise<boolean> => {
    let definition = input.definition;
    if (!input.isNew && input.complete === false) {
      const full = await migrateAndLoadFullDefinition(input.name);
      if (!full) return false;
      definition = { ...full, ...definition };
    }
    const args = [
      "--layer", "local",
      "--key", `skills.repositories.${input.name}`,
      "--value", JSON.stringify({ ...definition, _delete: false, _replace: true }),
    ];
    if (!(await writePolicy(args))) return false;
    void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    if (!input.silent) toast.success(t("来源已保存：{0}", input.name), {
      description: t("保存不会自动应用；需要时请在来源页更新或重新扫描。"),
    });
    return true;
  };

  return { saveDefinition };
}
