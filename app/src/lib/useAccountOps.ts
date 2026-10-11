import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { assistantExtraArgs } from "./aliasCommand";
import type { ProfileObserved, StatusData } from "./contract";
import { runEngine } from "./engine";
import { useExecutions } from "./execution";
import {
  buildProfileDefinition,
  type ProfileDraftPreview,
  type ProfileFormInput,
} from "./profileDefinition";
import { ALL_SEED_ITEMS, seedArgs, summarizeSeed, type SeedResult } from "./profileSeed";
import { engineKeys } from "./queries";
import { fillRows } from "./skillFill";
import { mcpFillArgs, mcpFillRows } from "./mcpFill";
import type { McpTarget } from "./contract";
import { accountCommand } from "./accountCommand";
import { useAliasSource, type AliasFilePreview } from "./useAliasSource";
import { sameJson } from "./utils";

export interface CreatePreview {
  config: ProfileDraftPreview;
  alias: AliasFilePreview;
  launchValue: string;
}

/**
 * 账号的新建、编辑、移除（docs/MULTI-ACCOUNT-DESIGN.md P2）。
 *
 * 写入保护沿用实例域：新建前预览并在保存时重读比对，编辑带 status 给出的文件与定义基准，
 * 引擎拒绝时文件与库都不动。新账号默认带上主账号的全部共用设置（权限、状态栏、规则、MCP），
 * 启动命令由引擎绑定账号；仅沿用主账号命令末尾的 CLI 参数。
 */
export function useAccountOps() {
  const aliasSource = useAliasSource();
  const { execute } = useExecutions();
  const queryClient = useQueryClient();
  const refresh = () => queryClient.invalidateQueries({ queryKey: engineKeys.all });

  /** 新账号的默认启动参数：沿用主账号命令里的固定参数。 */
  const mainExtraArgs = (bin: string): string => {
    const source = aliasSource.source;
    const value = source?.aliases[bin]
      ?? source?.targets.flatMap((target) => target.external).find((line) => line.name === bin)?.value;
    return value ? assistantExtraArgs(value) : "";
  };

  const launchValue = async (input: ProfileFormInput, bin: string, draft = false) => {
    const command = await accountCommand(`profiles:${input.id}`, "launch", "posix", draft ? buildProfileDefinition(input, bin) : undefined);
    return `${command.launch_value}${input.extraArgs.trim() ? ` ${input.extraArgs.trim()}` : ""}`;
  };

  const previewCreate = async (input: ProfileFormInput, bin: string): Promise<CreatePreview | string> => {
    try {
      const value = await launchValue(input, bin, true);
      const [config, alias] = await Promise.all([
        runEngine<ProfileDraftPreview>("profiles:preview", [
          "--only", `profiles:${input.id}`,
          "--definition", JSON.stringify(buildProfileDefinition(input, bin)),
          "--on-save",
          "--items", ALL_SEED_ITEMS.join(","),
        ]),
        aliasSource.previewSetAlias(input.id, value),
      ]);
      if (!config.ok) return config.errors[0]?.message ?? "配置预览失败";
      if (typeof alias === "string") return alias;
      return { config: config.data, alias, launchValue: value };
    } catch (error) {
      return `预览失败：${String(error)}`;
    }
  };

  /** 启动命令名不能和已有的助手命令或任何 alias 重名。 */
  const launchNameError = async (id: string, reserved: string[]): Promise<string | null> => {
    try {
      const result = await runEngine<StatusData>("aliases:status", []);
      if (!result.ok || result.data.elements.length === 0) return "读不到现有终端命令，没有保存；刷新后重试";
      const targets = result.data.elements.map((element) => element.observed as { blocked_reason?: string | null; external?: { name: string }[] });
      if (targets.some((target) => target.blocked_reason)) return "有终端配置文件读不了，无法确认命令名是否已被占用";
      const taken = reserved.includes(id)
        || result.data.aliases?.some((alias) => alias.name === id)
        || targets.some((target) => target.external?.some((alias) => alias.name === id));
      return taken ? `命令 ${id} 已存在，换一个名字` : null;
    } catch {
      return "读不到现有终端命令，没有保存；刷新后重试";
    }
  };

  /** 新账号拿到全部在用的技能（按技能逐个 add，单个失败不影响其余）。 */
  const linkSkills = async (agent: string): Promise<{ linked: number; failed: number }> => {
    const status = await runEngine<StatusData>("skills:status", []).catch(() => null);
    if (!status?.ok) return { linked: 0, failed: 0 };
    let linked = 0;
    let failed = 0;
    for (const row of fillRows(status.data.elements, [agent])) {
      const result = await execute({
        command: "skills:apply",
        label: row.label,
        args: ["--only", row.elementId, "--action", row.action, "--agent", row.agent],
        silent: true,
        refresh: false,
      });
      if (result?.ok) linked += 1;
      else failed += 1;
    }
    return { linked, failed };
  };

  /** 新账号拿到同类账号上在用的 MCP（主账号的已由共用设置带上，这里补其他账号上才有的）。 */
  const linkMcp = async (agent: string): Promise<{ copied: number; failed: number }> => {
    const status = await runEngine<StatusData & { targets?: McpTarget[] }>("mcp:status", []).catch(() => null);
    if (!status?.ok) return { copied: 0, failed: 0 };
    let copied = 0;
    let failed = 0;
    for (const row of mcpFillRows(status.data.elements, status.data.targets ?? [], [agent])) {
      const result = await execute({ command: "mcp:apply", label: `补齐 ${row.name} @ ${agent}`, args: mcpFillArgs(row), silent: true, refresh: false });
      if (result?.ok) copied += 1;
      else failed += 1;
    }
    return { copied, failed };
  };

  const create = async (
    input: ProfileFormInput,
    bin: string,
    expected: CreatePreview,
    reservedNames: string[],
    secret?: { variable: string; value: string },
  ): Promise<boolean> => {
    const nameError = await launchNameError(input.id, reservedNames);
    if (nameError) {
      toast.error(nameError);
      return false;
    }
    const again = await previewCreate(input, bin);
    if (typeof again === "string") {
      toast.error(`没有保存：${again}`);
      return false;
    }
    if (!sameJson(again, expected)) {
      toast.error("目标文件在预览后变了，没有保存；请看一眼新的预览再创建");
      return false;
    }
    if (!again.config.base) {
      toast.error("没有取得账号目录的当前内容，没有保存；请刷新后重试");
      return false;
    }
    const saved = await runEngine("profiles:create", [
      "--only", `profiles:${input.id}`,
      "--definition", JSON.stringify(buildProfileDefinition(input, bin)),
      "--base", again.config.base,
      ...(secret?.value ? ["--secret-var", secret.variable, "--secret-value", secret.value] : []),
    ]).catch((error) => ({ ok: false, errors: [{ message: String(error) }] }) as const);
    if (!saved.ok) {
      toast.error(`没有保存：${saved.errors[0]?.message ?? "未知错误"}`);
      return false;
    }
    let aliasOk = false;
    try {
      aliasOk = await aliasSource.setAlias(input.id, await launchValue(input, bin), { label: `写入账号 ${input.id} 的启动命令`, silent: true });
    } catch (error) {
      toast.error(`已创建账号 ${input.id}，但启动检查未通过`, { description: String(error) });
    }
    const seeded = await execute({
      command: "profiles:seed",
      args: seedArgs(input.id, ALL_SEED_ITEMS),
      label: `把共用设置带到账号 ${input.id}`,
      silent: true,
      refresh: false,
    });
    const seedResults = (seeded?.data as { results?: SeedResult[] } | null)?.results ?? [];
    const seedFailed = !seeded?.ok || summarizeSeed(seedResults).failed;
    const skills = await linkSkills(input.id);
    const mcp = await linkMcp(input.id);
    if (!aliasOk) toast.error(`已创建账号 ${input.id}，但启动命令没写进终端配置`, { description: "原因见任务记录。" });
    else if (seedFailed) toast.warning(`已创建账号 ${input.id}，部分共用设置没带上`, { description: summarizeSeed(seedResults).description || undefined });
    else if (skills.failed > 0) toast.warning(`已创建账号 ${input.id}，${skills.failed} 个技能没链接上`, { description: "到技能页点「补齐」重试，原因见任务记录。" });
    else if (mcp.failed > 0) toast.warning(`已创建账号 ${input.id}，${mcp.failed} 个 MCP 服务没复制上`, { description: "到 MCP 页点「补齐」重试，原因见任务记录。" });
    else toast.success(`已创建账号 ${input.id}`, { description: `已链接 ${skills.linked} 个技能；新开终端后输入 ${input.id} 启动。` });
    await refresh();
    return true;
  };

  const refreshLaunchBinding = async (id: string): Promise<void> => {
    const existing = aliasSource.source?.aliases[id];
    if (!existing) return;
    try {
      const current = await accountCommand(`profiles:${id}`, "launch");
      const extra = assistantExtraArgs(existing);
      const value = `${current.launch_value}${extra ? ` ${extra}` : ""}`;
      const ok = await aliasSource.setAlias(id, value, { label: `更新账号 ${id} 的启动命令`, silent: true });
      if (!ok) toast.warning(`账号 ${id} 已保存，启动命令需要更新`, { description: "到账号的「命令」里点「更新启动命令」，再新开终端。" });
    } catch (error) {
      toast.warning(`账号 ${id} 已保存，启动命令需要更新`, { description: String(error) });
    }
  };

  /** 只改显示名：只写登记，不碰目录文件。 */
  const rename = async (id: string, displayName: string, definition: Record<string, unknown>): Promise<boolean> => {
    const result = await runEngine("policy:set", [
      "--layer", "local",
      "--key", `agents.definitions.${id}`,
      "--value", JSON.stringify({ ...definition, display_name: displayName, _delete: false, _replace: true }),
      "--expect-definition", JSON.stringify(definition),
    ]).catch((error) => ({ ok: false, errors: [{ message: String(error) }] }) as const);
    if (!result.ok) toast.error(`没有保存：${result.errors[0]?.message ?? "未知错误"}`);
    else await refresh();
    return result.ok;
  };

  /**
   * 改接入参数：登记与目录里的配置文件一起写（profiles:apply-definition），
   * 保留打开表单时的文件与登记基准；精确预览失败前不保存新密钥。
   */
  const updateConnection = async (
    id: string,
    input: ProfileFormInput,
    bin: string,
    observed: ProfileObserved,
    secret?: { variable: string; value: string },
    displayName?: string,
  ): Promise<boolean> => {
    if (!observed?.base || !observed.definition_base) {
      toast.error("读不到这个账号目录的当前内容，没有保存");
      return false;
    }
    const profile = buildProfileDefinition(input, bin).profile as Record<string, unknown>;
    const keys = input.kind === "claude" ? ["env", "model"] : ["base_url", "env_key", "wire_api", "model"];
    const candidate: Record<string, unknown> = {};
    if (displayName !== undefined) candidate.display_name = displayName || id;
    for (const key of keys) if (profile[key] !== undefined) candidate[key] = profile[key];
    const bases = ["--only", `profiles:${id}`, "--base", observed.base, "--definition-base", observed.definition_base];
    const preview = await runEngine<ProfileDraftPreview>("profiles:preview", [...bases, "--candidate", JSON.stringify(candidate), "--exact",
      ...(secret?.value ? ["--preview-secret-var", secret.variable, "--preview-secret-value", secret.value] : [])]);
    if (!preview.ok || !preview.data.file_base || !preview.data.content_base) {
      toast.error(`没有保存：${preview.ok ? "预览没有完成，请重试" : preview.errors[0]?.message ?? "预览失败"}`);
      return false;
    }
    const applied = await execute({
      command: "profiles:apply-definition",
      label: `更新账号 ${id}`,
      args: [...bases, "--candidate", JSON.stringify(candidate),
        "--preview-file-base", preview.data.file_base, "--preview-content-base", preview.data.content_base,
        ...(secret?.value ? ["--secret-var", secret.variable, "--secret-value", secret.value] : [])],
      silent: true,
    });
    if (!applied?.ok) {
      const reason = applied?.errors[0]?.message ?? "详见任务记录";
      toast.error(`没有保存：${reason}`);
      await refresh();
      return false;
    }
    await refreshLaunchBinding(id);
    await refresh();
    return true;
  };

  /** 移除只停止管理并删掉启动命令；目录（含登录与会话）原样保留。 */
  const remove = async (id: string, definition: Record<string, unknown>): Promise<boolean> => {
    const result = await runEngine("policy:set", [
      "--layer", "local",
      "--key", `agents.definitions.${id}`,
      "--value", JSON.stringify({ _delete: true }),
      "--expect-definition", JSON.stringify(definition),
    ]).catch((error) => ({ ok: false, errors: [{ message: String(error) }] }) as const);
    if (!result.ok) {
      toast.error(`没有移除：${result.errors[0]?.message ?? "未知错误"}`);
      return false;
    }
    const commandRemoved = !aliasSource.source?.aliases[id]
      || await aliasSource.removeAlias(id, { label: `移除账号 ${id} 的启动命令`, silent: true });
    if (commandRemoved) {
      toast.success(`已移除账号 ${id}`, { description: "目录与里面的登录、会话都还在。" });
    } else {
      toast.warning(`账号 ${id} 已移除，启动命令未删除`, {
        description: "到「命令」页检查后删除；目录、登录与会话仍保留。",
      });
    }
    await refresh();
    return true;
  };

  /** 账号还没有启动命令：按登记补写一条。 */
  const addLaunch = async (id: string, _definition: Record<string, unknown>, _bin: string): Promise<boolean> => {
    try {
      const command = await accountCommand(`profiles:${id}`, "launch");
      const ok = await aliasSource.setAlias(id, command.launch_value, { label: `写入账号 ${id} 的启动命令` });
      if (ok) await refresh();
      return ok;
    } catch (error) {
      toast.error(`没有写入启动命令：${String(error)}`);
      return false;
    }
  };

  /** 目录被外部改过：以目录里的实际内容更新登记（只改库，文件不动）。 */
  const adopt = async (elementId: string, base: string): Promise<boolean> => {
    const result = await execute({
      command: "profiles:adopt",
      label: `以目录内容更新账号 ${elementId.replace(/^profiles:/, "")}`,
      args: ["--only", elementId, "--base", base],
    });
    if (result?.ok) await refreshLaunchBinding(elementId.replace(/^profiles:/, ""));
    await refresh();
    return result?.ok === true;
  };

  return { aliasSource, previewCreate, addLaunch, adopt, create, rename, updateConnection, remove, mainExtraArgs, launchValue, ready: Boolean(aliasSource.source) };
}
