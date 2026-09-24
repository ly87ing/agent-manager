import { useState } from "react";
import { Loader2 } from "lucide-react";
import { authMethodLabel, type OfficialAuthStatus } from "../lib/observed";
import type { ProfileKind } from "../lib/profileDefinition";
import { AuthCommandActions } from "./AuthCommandActions";

/**
 * 官方账号登录状态一行：按需用官方命令查一次，外加只复制不执行的登录 / 退出命令。
 *
 * 主配置与独立环境共用；主配置不带配置目录。查询结果只有是否登录与登录方式，不含账号或令牌。
 */
export function OfficialAuthPanel({
  kind,
  bin,
  configDir,
  thirdParty = false,
  testIds,
  onCheck,
}: {
  kind: ProfileKind;
  bin: string;
  /** 独立环境的配置目录；主配置不传。 */
  configDir?: string;
  /** 用第三方接口的环境：不给登录命令，查到官方登录时提示混用。 */
  thirdParty?: boolean;
  testIds: { root: string; text: string; check: string; mixed: string; commands: string };
  /** 用官方命令查一次登录状态；失败时返回原因。 */
  onCheck: () => Promise<OfficialAuthStatus | string>;
}) {
  const [auth, setAuth] = useState<OfficialAuthStatus | string | null>(null);
  const [checking, setChecking] = useState(false);
  const check = async () => {
    setChecking(true);
    try {
      setAuth(await onCheck());
    } finally {
      setChecking(false);
    }
  };
  const method = typeof auth === "object" ? authMethodLabel(auth?.method ?? null) : null;
  const text = auth === null
    ? "未检查（点「检查登录」用官方命令查询）"
    : typeof auth === "string"
      ? `检查失败：${auth}`
      : auth.state === "logged-in"
        ? `已登录${method ? `（${method}）` : ""}`
        : auth.state === "logged-out"
          ? "未登录"
          : `无法确认：${auth.reason ?? "未知原因"}`;

  return (
    <div
      data-testid={testIds.root}
      className="space-y-2 rounded-md border bg-muted/30 px-3 py-2 text-xs text-muted-foreground"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span>
          官方登录：
          <span data-testid={testIds.text} className="ml-1 font-medium text-foreground">{text}</span>
        </span>
        <div className="flex flex-wrap items-center gap-1.5">
          <button
            type="button"
            data-testid={testIds.check}
            disabled={checking}
            onClick={() => void check()}
            className="inline-flex items-center gap-1 rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            {checking && <Loader2 className="h-3 w-3 animate-spin" aria-hidden />}
            {checking ? "正在检查…" : "检查登录"}
          </button>
          <AuthCommandActions
            kind={kind}
            bin={bin}
            configDir={configDir}
            testIdPrefix={testIds.commands}
            allowLogin={!thirdParty}
          />
        </div>
      </div>
      {thirdParty && (
        <p className="text-[11px]">这个环境用第三方接口，不要在这里登录官方账号；官方账号请用主配置或单独的官方账号环境。</p>
      )}
      {typeof auth === "object" && auth?.mixed && (
        <p data-testid={testIds.mixed} role="alert" className="rounded border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-destructive">
          这个环境用第三方接口，却有官方账号登录：两种凭据混在同一个配置目录。请复制退出命令在终端里对这个环境退出登录，或给这套第三方接入换一个新目录。
        </p>
      )}
    </div>
  );
}
