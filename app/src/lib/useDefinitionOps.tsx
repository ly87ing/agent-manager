import { t } from "./i18n";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import type { PolicyData } from "./contract";
import { engineErrorMessage, runEngine } from "./engine";
import { engineKeys } from "./queries";
import { TOGGLE_COPY } from "./ui-copy";

/** Save a skill source definition without installing or applying its resources. */
export function useDefinitionOps() {
  const queryClient = useQueryClient();

  const writePolicy = async (args: string[]): Promise<boolean> => {
    let envelope;
    try {
      envelope = await runEngine<PolicyData>("policy:set", args);
    } catch (err) {
      toast.error(`${TOGGLE_COPY.saveFailed}:${engineErrorMessage(err)}`);
      return false;
    }
    if (!envelope.ok) {
      toast.error(`${TOGGLE_COPY.saveRejected}:${envelope.errors[0]?.message ?? t("未知错误")}`);
      return false;
    }
    return true;
  };

  /** A legacy definition may need migration before its complete stored value can be edited. */
  const migrateAndLoadFullDefinition = async (name: string): Promise<Record<string, unknown> | null> => {
    const fail = (message: string) => {
      toast.error(t("保存定义 {0} 失败:{1}", name, message));
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
    toast.success(t("已保存定义 {0} 到本地资源库（不会自动安装或同步）", input.name));
    return true;
  };

  return { saveDefinition };
}
