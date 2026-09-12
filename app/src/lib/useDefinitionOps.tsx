import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import type { PolicyData, PolicyLayers } from "./contract";
import { definitionKeyPrefix, planDefinitionRemoval, type DefinitionSection } from "./definitionOverlay";
import { engineErrorMessage, runEngine } from "./engine";
import { engineKeys } from "./queries";
import { OVERLAY_COPY, TOGGLE_COPY } from "./ui-copy";
import { useExecuteApply, usePlanPreview } from "./usePlanPreview";

/**
 * 定义覆盖层的新增、编辑与移除；Agent 移除仅停止管理，
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
    const defsKey = section === "mcp" ? "definitions" : "repositories";
    const defs = sectionData?.[defsKey] as Record<string, unknown> | undefined;
    const full = defs?.[name];
    if (typeof full !== "object" || full === null) {
      return fail("初始化后仍取不到完整定义，已停止保存（定义未改动）");
    }
    return full as Record<string, unknown>;
  };

  /** 新增/编辑：写定义。Agent 与资源定义（MCP/技能集合）只保存定义，
   * 不写启用键、不 plan/apply、不触发安装/同步。
   * 资源定义始终整条替换（_replace）：编辑时若调用方拿不到完整原定义（无 DB 的旧存储），
   * 先 db:migrate 再取 sqlite 有效定义合并表单字段，避免丢掉未展示字段或复活已清空字段。 */
  const saveAndApply = async (input: {
    name: string;
    definition: Record<string, unknown>;
    /** true = 新增；false = 编辑（不动启用键）。 */
    isNew: boolean;
    /** 预览过滤的元素 id（mcp:<name> / skill:<name>）。 */
    elementId: string;
    /** definition 是否为完整整条定义（mcp/skills 用）。新增恒为完整；编辑时调用方有
     * sqlite 有效定义则传完整定义（complete 可缺省），拿不到完整原定义必须显式传 false。 */
    complete?: boolean;
  }): Promise<void> => {
    const defKey = `${definitionKeyPrefix(section)}.${input.name}`;
    let definition = input.definition;
    if (domain !== "agents" && !input.isNew && input.complete === false) {
      const full = await migrateAndLoadFullDefinition(input.name);
      if (!full) return;
      definition = { ...full, ...definition };
      if (domain === "mcp") {
        // 传输互斥剥离：以表单提交的 owned 片段判定目标传输（远程含 url），
        // 合并后 full 中遗留的对方传输字段必须剥掉，避免混合定义被引擎拒绝。
        // type 为历史遗留输入，定义不持久化，两种形态保存时一律剥离。
        const remoteIntent = typeof input.definition.url === "string";
        const stripKeys = remoteIntent
          ? ["command", "args", "env", "startup_timeout_sec", "extra_args_by_agent", "type"]
          : ["url", "headers", "type"];
        for (const key of stripKeys) {
          delete definition[key];
        }
        // spec.env / spec.headers 是脱敏视图：普通字面值被掩码为 <literal>。表单只有脱敏
        // 片段时会带出 <literal>，这里用完整定义的真实值重建保留变量，不让 <literal> 覆盖
        // 真实值。若在完整 DB 原定义找不到对应 string，必须 toast 失败并停止保存，
        // 绝不把脱敏占位符存回 DB。env（stdio）与 headers（远程）同级处理。
        for (const field of ["env", "headers"] as const) {
          if (!(field in definition)) continue;
          const fullMap = (full[field] ?? {}) as Record<string, unknown>;
          const ownedMap = (definition[field] ?? {}) as Record<string, unknown>;
          const rebuilt: Record<string, unknown> = {};
          for (const [key, value] of Object.entries(ownedMap)) {
            if (value === "<literal>") {
              const raw = fullMap[key];
              if (typeof raw !== "string") {
                toast.error(`保存定义 ${input.name} 失败:${field === "env" ? "环境变量" : "请求头"} ${key} 无法在原定义中还原真实值，已停止保存`);
                return;
              }
              rebuilt[key] = raw;
            } else {
              rebuilt[key] = value;
            }
          }
          definition = { ...definition, [field]: rebuilt };
        }
      }
    }
    if (domain === "mcp") {
      for (const field of ["env", "headers"] as const) {
        const map = definition[field];
        if (map && typeof map === "object") {
          for (const [key, value] of Object.entries(map as Record<string, unknown>)) {
            if (value === "<literal>") {
              toast.error(`保存定义 ${input.name} 失败:${field === "env" ? "环境变量" : "请求头"} ${key} 仍为脱敏值，已停止保存`);
              return;
            }
          }
        }
      }
    }
    if (!(await writePolicy("policy:set", ["--layer", "local", "--key", defKey, "--value", JSON.stringify({ ...definition, _delete: false, _replace: true })]))) {
      return;
    }
    // 表单提交语义为「保存定义」：只保存定义，不写启用键、不 plan/apply、不触发安装或
    // 同步；_replace 整条替换保证清空的可选字段不会复活。
    void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    toast.success(
      domain === "agents"
        ? `已保存定义 ${input.name}（不会自动安装）`
        : `已保存定义 ${input.name} 到本地资源库（不会自动安装或同步）`,
    );
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
      const elementId = `skill:${name}`;
      preview.open({
        title: `移除 ${name}`,
        subtitle: OVERLAY_COPY.removeDefinitionNote,
        planCommands: [{ command: "skills:plan", args: ["--only", elementId, "--action", "remove"] }],
        confirmLabel: TOGGLE_COPY.applyNow,
        execute: executeApply({
          command: "skills:apply",
          label: `移除 ${name}`,
          extraArgs: ["--only", elementId, "--action", "remove"],
        }),
      });
      return;
    }
    setRemovalExecuting(true);
    const plan = planDefinitionRemoval(removal.layers, section, removal.name);
    const ok = await writePolicy("policy:set", ["--layer", plan.layer, "--key", plan.key, "--value", '{"_delete": true}']);
    setRemovalExecuting(false);
    if (!ok) return;
    const name = removal.name;
    setRemoval(null);
    void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    toast.success(`已移除 ${name}`);
    if (domain === "agents") return;
    const prefix = domain;
    const elementId = `${prefix}:${name}`;
    preview.open({
      title: `移除 ${name}`,
      subtitle: OVERLAY_COPY.removeDefinitionNote,
      planCommands: [{ command: `${domain}:plan`, args: ["--only", elementId] }],
      confirmLabel: TOGGLE_COPY.applyNow,
      execute: executeApply({
        command: `${domain}:apply`,
        label: `移除 ${name}`,
        extraArgs: ["--only", elementId],
      }),
    });
  };

  const removalPlan = removal ? planDefinitionRemoval(removal.layers, section, removal.name) : null;
  const removalModal = removal && removalPlan ? (
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

  return { saveAndApply, requestRemove, modals: (
    <>
      {removalModal}
      {preview.modal}
    </>
  ) };
}
