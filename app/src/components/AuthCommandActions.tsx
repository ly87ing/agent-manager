import { LogIn, LogOut } from "lucide-react";
import { toast } from "sonner";
import { authTerminalCommand, type ProfileKind } from "../lib/profileDefinition";

export function AuthCommandActions({
  kind,
  bin,
  configDir,
  testIdPrefix,
  allowLogin = true,
}: {
  kind: ProfileKind;
  bin: string;
  configDir?: string;
  testIdPrefix: string;
  /** 第三方环境不给登录命令：在那里登录官方账号就是两种凭据混用。 */
  allowLogin?: boolean;
}) {
  const copy = async (action: "login" | "logout") => {
    try {
      await navigator.clipboard.writeText(authTerminalCommand(kind, bin, action, configDir));
      toast.success(`已复制${action === "login" ? "登录" : "退出"}命令`, {
        description: "应用没有执行它；请确认目标账号与配置目录后，在终端里亲自运行。",
      });
    } catch {
      toast.error("复制失败，请检查剪贴板权限");
    }
  };

  return (
    <div className="flex flex-wrap items-center gap-1.5" aria-label="认证操作">
      {allowLogin && (
        <button
          type="button"
          data-testid={`${testIdPrefix}-auth-login`}
          onClick={() => void copy("login")}
          className="inline-flex items-center gap-1 rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent"
        >
          <LogIn className="h-3 w-3" aria-hidden /> 复制登录命令
        </button>
      )}
      <button
        type="button"
        data-testid={`${testIdPrefix}-auth-logout`}
        onClick={() => void copy("logout")}
        className="inline-flex items-center gap-1 rounded border border-destructive/30 px-2 py-1 text-xs text-destructive hover:bg-destructive/10"
      >
        <LogOut className="h-3 w-3" aria-hidden /> 复制退出命令
      </button>
      <span className="text-[11px] text-muted-foreground">只复制，不执行</span>
    </div>
  );
}
