import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { engineErrorMessage, runEngine } from "./engine";
import { engineKeys } from "./queries";

/**
 * 终端快捷命令来源（aliases.shell）的读取与保存。
 *
 * 保存走 sources:set 的 revision 比对：读到什么版本就带什么版本写回，期间被别处改过即失败，
 * 不覆盖他人改动。未展示字段（如 _comment）以读到的原值为基底保留。
 * 保存只写本地资源库，不碰 shell 配置文件——写入 ~/.zshrc / ~/.zprofile 要另点「立即同步」。
 */

export const DEFAULT_TIMEZONE = "America/Los_Angeles";
export const DEFAULT_LANG = "en_US.UTF-8";

interface RawSource {
  key: string;
  label: string;
  value: string | Record<string, unknown>;
  revision: string;
  format: "text" | "json";
}

export interface AliasSource {
  key: string;
  revision: string;
  timezone: string;
  lang: string;
  aliases: Record<string, string>;
  /** 读到的整条原值，保存时作为基底。 */
  raw: Record<string, unknown>;
}

function toAliasSource(source: RawSource): AliasSource {
  const raw = (typeof source.value === "object" && source.value !== null ? source.value : {}) as Record<string, unknown>;
  const aliases = (raw.aliases as Record<string, string> | undefined) ?? {};
  return {
    key: source.key,
    revision: source.revision,
    timezone: typeof raw.timezone === "string" ? raw.timezone : DEFAULT_TIMEZONE,
    lang: typeof raw.lang === "string" ? raw.lang : DEFAULT_LANG,
    aliases: { ...aliases },
    raw,
  };
}

export function useAliasSource() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: [...engineKeys.command("sources:get"), "aliases"],
    queryFn: () => runEngine<{ sources: RawSource[] }>("sources:get", ["--key", "aliases"]),
  });

  const source = query.data?.ok
    ? (() => {
        const entry = query.data.data.sources.find((item) => item.key === "aliases.shell") ?? query.data.data.sources[0];
        return entry ? toAliasSource(entry) : null;
      })()
    : null;

  /** 以当前来源为基底改写后整条保存；成功返回 true。 */
  const save = async (
    next: { timezone?: string; lang?: string; aliases?: Record<string, string> },
    successMessage: string,
    /** silent: 由调用方给更合适的提示（例如带撤销的那种）。 */
    options?: { silent?: boolean },
  ): Promise<boolean> => {
    if (!source) {
      toast.error("保存失败:还没读到当前配置");
      return false;
    }
    const value = {
      ...source.raw,
      timezone: next.timezone ?? source.timezone,
      lang: next.lang ?? source.lang,
      aliases: next.aliases ?? source.aliases,
    };
    let envelope;
    try {
      envelope = await runEngine("sources:set", [
        "--key",
        source.key,
        "--value",
        JSON.stringify({ revision: source.revision, value }),
      ]);
    } catch (err) {
      toast.error(`保存失败:${engineErrorMessage(err)}`);
      return false;
    }
    if (!envelope.ok) {
      toast.error(`保存被拒绝:${envelope.errors[0]?.message ?? "未知错误"}`);
      return false;
    }
    void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    if (!options?.silent) toast.success(successMessage);
    return true;
  };

  return { query, source, save };
}
