import { t } from "../lib/i18n";
import { Copy } from "lucide-react";
import { toast } from "sonner";

function Example({ label, text }: { label: string; text: string }) {
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      toast.success(t("已复制示例，请替换 YOUR_* 后自行检查；没有执行命令"));
    } catch {
      toast.error(t("复制失败，可直接选中示例文字复制"));
    }
  };

  return (
    <div className="space-y-2 rounded-md bg-muted p-3">
      <div className="flex items-center justify-between gap-3">
        <span className="text-xs font-medium">{label}</span>
        <button type="button" aria-label={t("复制{0}", label)} onClick={() => void copy()} className="inline-flex shrink-0 items-center gap-1 rounded border bg-card px-2 py-1 text-xs hover:bg-accent">
          <Copy className="h-3 w-3" aria-hidden />{t("复制示例")}</button>
      </div>
      <pre className="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed"><code>{text}</code></pre>
    </div>
  );
}

/** Bundled guidance only. Do not add engine, filesystem or process access here. */
export default function TipsPage() {
  return (
    <div className="space-y-3" data-testid="tips-page">
      <h1 className="text-lg font-semibold tracking-tight">{t("更多技巧")}</h1>
      <p className="text-sm text-muted-foreground">{t("分享多账号与终端别名的使用经验。这里只展示文字，复制示例不会执行命令或修改文件。")}</p>

      <article aria-labelledby="tips-environments" className="space-y-3 rounded-lg border bg-card p-4">
        <h2 id="tips-environments" className="text-sm font-semibold">{t("多账号与独立环境")}</h2>
        <p className="text-sm">{t("准备多个使用环境时，先核对官方工具支持的隔离方式，再分别准备配置、登录存储和会话位置。")}</p>
        <pre className="rounded-md bg-muted p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap break-words"><code>{"YOUR_ENV_A/\n  YOUR_CONFIG_DIR/\n  YOUR_SESSION_DIR/\nYOUR_ENV_B/\n  YOUR_CONFIG_DIR/\n  YOUR_SESSION_DIR/"}</code></pre>
        <p className="text-xs text-muted-foreground">{t("目录名仅作示意，实际位置按官方文档确认。独立目录或不同别名本身不能证明登录已经隔离。")}</p>
        <ol className="list-decimal space-y-2 pl-5 text-sm">
          <li>{t("分别使用官方工具完成登录与身份确认，不把一个环境的凭据、登录文件或会话复制到另一个环境。")}</li>
          <li>{t("检查启动终端继承的密钥、服务地址等环境变量，避免两个环境无意中使用同一组取值。")}</li>
          <li>{t("每次使用前确认当前环境的身份，不根据另一个窗口的登录状态作判断。")}</li>
        </ol>
        <p className="text-xs text-muted-foreground">{t("本页不检测账号、不生成启动命令，也不创建目录、登录或切换账号。")}</p>
      </article>

      <article aria-labelledby="tips-aliases" className="space-y-3 rounded-lg border bg-card p-4">
        <h2 id="tips-aliases" className="text-sm font-semibold">{t("终端别名")}</h2>
        <p className="text-sm">{t("给已经检查过的命令取一个短名字。别名只缩短输入，不负责账号或环境隔离。")}</p>
        <p className="text-xs text-muted-foreground">{t("以下示例适用于 Bash / Zsh；PowerShell 和 cmd 使用不同语法。YOUR_* 需要由你替换。")}</p>
        <Example label={t("别名示例")} text="alias YOUR_SHORTCUT='YOUR_COMMAND'" />
        <p className="text-sm">{t("需要明确传递参数时，可以使用函数；")}<code className="font-mono text-xs">{"\"$@\""}</code>{(" " + t("保留每个参数的边界。"))}</p>
        <Example label={t("函数示例")} text={'YOUR_SHORTCUT() {\n  YOUR_COMMAND "$@"\n}'} />
        <ul className="list-disc space-y-2 pl-5 text-sm">
          <li>{(t("取名之前，可在终端用") + " ")}<code className="font-mono text-xs">type YOUR_SHORTCUT</code>{(" " + t("查看是否已有同名命令。"))}</li>
          <li>{t("在终端直接定义只对当前 shell 生效。需要长期保留时，由你检查后自行编辑对应的 shell 启动文件。")}</li>
          <li>{t("如果简称指向某个独立环境，应先确认原命令的配置、登录和环境变量都属于那个环境。")}</li>
        </ul>
        <p className="text-xs text-muted-foreground">{t("本页不读取或保存别名，不编辑 shell 文件，也没有运行示例的入口。")}</p>
      </article>
    </div>
  );
}
