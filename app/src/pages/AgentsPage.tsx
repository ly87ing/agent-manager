import { t } from "../lib/i18n";
import { Bot } from "lucide-react";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { useEngineQuery } from "../lib/queries";
import type { ResourceTarget } from "../lib/useAccountNames";

export default function AgentsPage() {
  const query = useEngineQuery<{ targets: ResourceTarget[] }>("resources:targets");
  if (query.isPending) return <PageLoading text={t("正在读取助手列表…")} />;
  if (query.isError) return <PageError title={t("助手列表加载失败")} error={query.error} />;
  if (!query.data.ok) return <ErrorPanel title={t("助手列表加载失败")} errors={query.data.errors} />;
  const targets = query.data.data.targets ?? [];

  return (
    <div className="space-y-4">
      <PageHeader title={t("助手")} description={t("只读查看已有扩展目标。记录来自内置目录与已有标签，不代表已安装或已登录。")} />
      <p className="text-xs text-muted-foreground">{t("不读取登录身份或凭据，不执行助手命令，也不修改账号或配置。")}</p>
      {targets.length === 0 ? (
        <EmptyState icon={Bot} title={t("暂无助手目标")} description={t("这里仅展示已有记录，不会自动创建目录或账号。")} />
      ) : (
        <section aria-label={t("助手列表")} className="divide-y rounded-lg border bg-card">
          {targets.map((target) => {
            const parent = targets.find((item) => item.id === target.group)?.name ?? target.group;
            const paths = [
              [t("配置目录"), target.config_dir],
              [t("技能目录"), target.skills_dir],
              [t("MCP 文件"), target.mcp_path],
            ].filter((entry) => entry[1]);
            return (
              <article key={target.id} className="flex flex-wrap gap-x-6 gap-y-3 px-4 py-4">
                <div className="min-w-0 flex-1 basis-40">
                  <h3 className="break-words text-sm font-medium">{target.name}</h3>
                  <p className="mt-1 break-all text-xs text-muted-foreground">{target.id}</p>
                  {target.group !== target.id && <p className="mt-1 break-words text-xs text-muted-foreground">{parent}{(" " + t("· 已有目录标签"))}</p>}
                </div>
                <dl className="min-w-0 flex-[2] basis-80 space-y-2 text-xs">
                  {paths.map(([label, path]) => (
                    <div key={label} className="flex gap-3">
                      <dt className="w-16 shrink-0 text-muted-foreground">{label}</dt>
                      <dd className="min-w-0 break-all font-mono">{path}</dd>
                    </div>
                  ))}
                  {paths.length === 0 && <div className="text-muted-foreground">{t("未登记扩展路径")}</div>}
                </dl>
              </article>
            );
          })}
        </section>
      )}
    </div>
  );
}
