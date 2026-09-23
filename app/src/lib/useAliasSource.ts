import { useRef, useState } from "react";
import { toast } from "sonner";
import type { AliasCurrent, AliasLegacySuggestion, AliasRecommendation, EngineElement, StatusData } from "./contract";
import type { AliasesObserved, AliasesSpec, AliasLine } from "./observed";
import { useExecutions } from "./execution";
import { useEngineQuery } from "./queries";

/**
 * 终端快捷命令：以 ~/.zshrc 与 ~/.zprofile 里本工具那一段（受管区块）的实际行为准。
 *
 * 读取走 aliases:status；每次改动都是一次 aliases:apply，按名改写一条并直接写回文件，
 * 带上打开时读到的各文件基准（base），文件在此期间被别处改过即被拒绝，不覆盖。
 * 两个文件保持同一写法：能写的文件都带上，读写不了的那个跳过（页面上显示原因）。
 */

/** 新建快捷命令时预填的时区与语言（与内置推荐一致）。 */
export const DEFAULT_TIMEZONE = "America/Los_Angeles";
export const DEFAULT_LANG = "en_US.UTF-8";

export interface AliasTarget {
  id: string;
  /** ~/.zshrc 一类的展示路径。 */
  target: string;
  state: string;
  /** 能写时的写入基准；读写不了为 null。 */
  base: string | null;
  baseFlag: string;
  blockedReason: string | null;
  entries: AliasLine[];
  /** 区块外用户自己的别名：只读。 */
  external: AliasLine[];
}

export interface AliasSource {
  /** 名称 → 取值（本工具写法的那些，按名合并）。 */
  aliases: Record<string, string>;
  current: AliasCurrent[];
  targets: AliasTarget[];
  recommendations: AliasRecommendation[];
  legacy: AliasLegacySuggestion[];
  /** 至少有一个文件能写。 */
  writable: boolean;
}

function targetOf(element: EngineElement): AliasTarget {
  const observed = element.observed as unknown as AliasesObserved;
  const spec = element.spec as unknown as AliasesSpec;
  const writable = element.actions.some((action) => action.id === "write") && typeof observed.base === "string";
  return {
    id: element.id,
    target: spec.target ?? observed.target,
    state: element.state,
    base: writable ? observed.base : null,
    baseFlag: spec.base_flag,
    blockedReason: observed.blocked_reason ?? null,
    entries: observed.entries ?? [],
    external: observed.external ?? [],
  };
}

function toSource(data: StatusData): AliasSource {
  const current = data.aliases ?? [];
  const aliases: Record<string, string> = {};
  for (const item of current) {
    if (item.value !== null) aliases[item.name] = item.value;
  }
  const targets = data.elements.map(targetOf);
  return {
    aliases,
    current,
    targets,
    recommendations: data.alias_recommendations ?? [],
    legacy: data.alias_legacy ?? [],
    writable: targets.some((target) => target.base !== null),
  };
}

export function useAliasSource() {
  const query = useEngineQuery<StatusData>("aliases:status");
  const { execute } = useExecutions();
  const [busy, setBusy] = useState(false);
  const source = query.data?.ok ? toSource(query.data.data) : null;
  // 写入时取最新读到的基准：撤销等延迟回调里也不会拿着过期的 base。
  const latest = useRef<AliasSource | null>(source);
  latest.current = source;

  const run = async (
    command: string,
    args: string[],
    label: string,
    options?: { silent?: boolean },
  ): Promise<boolean> => {
    setBusy(true);
    try {
      const envelope = await execute({ command, args, label, silent: options?.silent });
      return envelope?.ok === true;
    } finally {
      setBusy(false);
    }
  };

  /** 按名改写一条（写进所有能写的文件）；成功返回 true。 */
  const write = async (op: string[], label: string, options?: { silent?: boolean }): Promise<boolean> => {
    const current = latest.current;
    const bases = (current?.targets ?? []).flatMap((target) => (target.base ? [target.baseFlag, target.base] : []));
    if (bases.length === 0) {
      toast.error(`${label} 失败：~/.zshrc 与 ~/.zprofile 当前都不能写，原因见「终端快捷命令」页`);
      return false;
    }
    return run("aliases:apply", [...bases, ...op], label, options);
  };

  const setAlias = (
    name: string,
    value: string,
    options?: { renameFrom?: string; label?: string; silent?: boolean },
  ): Promise<boolean> => {
    const renamed = options?.renameFrom && options.renameFrom !== name ? options.renameFrom : undefined;
    return write(
      ["--set", name, "--value", value, ...(renamed ? ["--rename-from", renamed] : [])],
      options?.label ?? (renamed ? `把 ${renamed} 改名为 ${name}` : `保存快捷命令 ${name}`),
      options,
    );
  };

  const removeAlias = (name: string, options?: { label?: string; silent?: boolean }): Promise<boolean> =>
    write(["--remove", name], options?.label ?? `移除快捷命令 ${name}`, options);

  const addRecommended = (name: string): Promise<boolean> =>
    write(["--recommended", name], `添加推荐的快捷命令 ${name}`);

  const addLegacy = (item: AliasLegacySuggestion): Promise<boolean> =>
    write(["--legacy", item.id], `加入已保存的快捷命令 ${item.name}`);

  /** 丢弃一条旧版本保存的快捷命令：只删本机库里的这一条，不改终端配置文件。 */
  const discardLegacy = (item: AliasLegacySuggestion): Promise<boolean> =>
    run("aliases:discard-legacy", ["--legacy", item.id], `丢弃已保存的快捷命令 ${item.name}`);

  return { query, source, busy, setAlias, removeAlias, addRecommended, addLegacy, discardLegacy };
}
