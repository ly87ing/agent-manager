import { useState } from "react";
import { RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { RecentSessions } from "../components/RecentSessions";
import { AgentIcon } from "../components/AgentIcon";
import { useAccountNames } from "../lib/useAccountNames";
import {
  byAccount,
  cacheHitRate,
  dailySeries,
  usageAccountKey,
  formatCost,
  filterUsage,
  groupUsage,
  priceOf,
  shortTokens,
  totalsOf,
  withinDays,
  type ModelPrice,
  type UsageData,
} from "../lib/usageModel";
import { useQueryClient } from "@tanstack/react-query";
import { cn } from "../lib/utils";
import { useDialogFocus } from "../lib/dialogFocus";

/**
 * 用量：从助手自己的会话记录里累加 token，按模型算花了多少。
 *
 * 数字来自本机文件，不联网。费用是按单价表估的，不等于账单——没有单价的模型
 * 只报 token 不报钱，宁可留空也不拿 0 冒充。
 */

/**
 * 填某个模型的单价。
 *
 * 整张 pricing 表一次写回，不走 `usage.pricing.<模型名>` 那条路径——模型名里的点号
 * （gpt-5.6-sol）会被 policy 的键路径当成分隔符，拆成 gpt-5 / 6-sol。
 */
function PriceDialog({
  model,
  current,
  pricing,
  onClose,
  onSaved,
}: {
  model: string;
  current: ModelPrice | null;
  pricing: Record<string, ModelPrice>;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [form, setForm] = useState({
    input: current?.input?.toString() ?? "",
    output: current?.output?.toString() ?? "",
    cache_write: current?.cache_write?.toString() ?? "",
    cache_read: current?.cache_read?.toString() ?? "",
  });
  const [saving, setSaving] = useState(false);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({
    onEscape: saving ? undefined : onClose,
  });

  const fields: { key: keyof typeof form; label: string }[] = [
    { key: "input", label: "输入" },
    { key: "output", label: "输出" },
    { key: "cache_write", label: "缓存写入" },
    { key: "cache_read", label: "缓存读取" },
  ];

  const submit = async () => {
    const entry: ModelPrice = {};
    for (const { key } of fields) {
      const raw = form[key].trim();
      if (!raw) continue;
      const value = Number(raw);
      if (!Number.isFinite(value) || value < 0) {
        toast.error(`${key} 要是非负数字`);
        return;
      }
      entry[key] = value;
    }
    setSaving(true);
    try {
      const next = { ...pricing, [model]: entry, _replace: true };
      const envelope = await runEngine("policy:set", [
        "--layer",
        "local",
        "--key",
        "usage.pricing",
        "--value",
        JSON.stringify(next),
      ]);
      if (!envelope.ok) {
        toast.error(`单价没存上:${envelope.errors[0]?.message ?? "未知错误"}`);
        return;
      }
      toast.success(`已记下 ${model} 的单价`);
      onSaved();
      onClose();
    } catch (err) {
      toast.error(`单价没存上:${engineErrorMessage(err)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4"
      onClick={(event) => event.target === event.currentTarget && onClose()}
    >
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-label={`${model} 的单价`} tabIndex={-1} onKeyDown={onDialogKeyDown} className="max-h-[calc(100vh-2rem)] w-full max-w-md overflow-y-auto rounded-xl border bg-card p-5 shadow-xl">
        <h3 className="text-base font-semibold text-foreground">{model} 的单价</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          美元 / 百万 token，去这家的定价页抄。留空的那项如果真有用量，这个模型就整体不折算——
          宁可不报也不报个错的数。
        </p>
        <div className="mt-4 grid grid-cols-2 gap-3">
          {fields.map(({ key, label }) => (
            <label key={key} className="block text-xs text-muted-foreground">
              {label}
              <input
                data-testid={`price-${key}`}
                value={form[key]}
                onChange={(event) => setForm((prev) => ({ ...prev, [key]: event.target.value }))}
                inputMode="decimal"
                placeholder="0.00"
                className="mt-1 w-full rounded border border-input p-1.5 font-mono text-xs"
              />
            </label>
          ))}
        </div>
        <div className="sticky -bottom-5 z-10 -mx-5 -mb-5 border-t bg-card px-5 py-3 mt-4 flex justify-end gap-2">
          <button type="button" onClick={onClose} disabled={saving} className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent disabled:opacity-40">
            取消
          </button>
          <button
            type="button"
            data-testid="price-submit"
            onClick={() => void submit()}
            disabled={saving}
            className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground disabled:opacity-50"
          >
            存下
          </button>
        </div>
      </div>
    </div>
  );
}

const RANGES: { days: number; label: string }[] = [
  { days: 1, label: "今天" },
  { days: 7, label: "近 7 天" },
  { days: 30, label: "近 30 天" },
  { days: 0, label: "全部" },
];

/** 趋势图里区分账号的颜色（按用量从多到少依次取）。 */
const SERIES_COLORS = ["hsl(var(--primary))", "hsl(var(--orange))", "hsl(var(--violet))", "hsl(var(--success))", "hsl(var(--rose))", "hsl(var(--info))"];

/** 每日趋势：一天一根柱，按账号叠起来；悬停看当天各账号的数。 */
function UsageTrend({
  series,
  agents,
  colorOf,
  labelOf,
}: {
  series: ReturnType<typeof dailySeries>;
  agents: string[];
  colorOf: (agent: string) => string;
  labelOf: (agent: string) => string;
}) {
  const max = Math.max(1, ...series.map((point) => point.total));
  const barGap = series.length > 31 ? 1 : 3;
  return (
    <section data-testid="usage-trend" className="rounded-lg border bg-card p-4">
      <div className="mb-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span className="font-medium text-foreground">每日用量</span>
        <span>输入 + 输出 token</span>
        <span className="ml-auto flex flex-wrap gap-3">
          {agents.map((agent) => (
            <span key={agent} className="inline-flex items-center gap-1.5">
              <span className="h-2 w-2 rounded-sm" style={{ background: colorOf(agent) }} aria-hidden />
              {labelOf(agent)}
            </span>
          ))}
        </span>
      </div>
      <div className="flex h-32 items-end justify-around" style={{ gap: barGap }}>
        {series.map((point) => (
          <div
            key={point.day}
            data-testid={`usage-trend-${point.day}`}
            title={`${point.day}：${shortTokens(point.total)}${agents
              .filter((agent) => point.byAgent[agent])
              .map((agent) => `\n${labelOf(agent)} ${shortTokens(point.byAgent[agent])}`)
              .join("")}`}
            className="flex h-full min-w-0 max-w-10 flex-1 flex-col-reverse overflow-hidden rounded-sm bg-muted/40"
          >
            {agents.map((agent) =>
              point.byAgent[agent] ? (
                <div key={agent} style={{ height: `${(point.byAgent[agent] / max) * 100}%`, background: colorOf(agent) }} />
              ) : null,
            )}
          </div>
        ))}
      </div>
      <div className="mt-1.5 flex justify-between text-[11px] tabular-nums text-muted-foreground">
        <span>{series[0]?.day.slice(5)}</span>
        <span>{series[series.length - 1]?.day.slice(5)}</span>
      </div>
    </section>
  );
}

export default function UsagePage() {
  const query = useEngineQuery<UsageData>("usage:status");
  const nameOf = useAccountNames();
  /** 账号全名：「Claude · 主号」「Claude · 工作」，与命令页一致。 */
  const labelOf = (agent: string) => {
    const { id, kind } = usageAccountKey(agent);
    if (!kind) return agent;
    const assistant = kind === "codex" ? "Codex" : "Claude";
    return id === "claude-code" || id === "codex" ? `${assistant} · 主号` : `${assistant} · ${nameOf(id)}`;
  };
  const queryClient = useQueryClient();
  const [days, setDays] = useState(7);
  const [agent, setAgent] = useState("");
  const [model, setModel] = useState("");
  const [scanning, setScanning] = useState(false);
  /** 正在填单价的模型。 */
  const [pricing, setPricing] = useState<string | null>(null);

  const rescan = async () => {
    setScanning(true);
    try {
      const envelope = await runEngine<{ files?: number; bytes?: number }>("usage:scan");
      if (!envelope.ok) {
        toast.error(`统计失败:${envelope.errors[0]?.message ?? "未知错误"}`);
        return;
      }
      const files = envelope.data?.files ?? 0;
      toast.success(files > 0 ? `读了 ${files} 个会话文件的新增部分` : "没有新的会话记录");
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    } catch (err) {
      toast.error(`统计失败:${engineErrorMessage(err)}`);
    } finally {
      setScanning(false);
    }
  };

  if (query.isPending) return <PageLoading text="正在读用量…" />;
  if (query.isError) return <PageError title="用量读取失败" error={query.error} />;

  const data = query.data.data;
  const timeGroups = withinDays(groupUsage(data), days);
  const agents = [...new Set(timeGroups.map((group) => group.agent))].sort();
  const agentGroups = filterUsage(timeGroups, agent, "");
  const models = [...new Set(agentGroups.map((group) => group.model))].sort();
  const groups = filterUsage(agentGroups, "", model);
  const totals = totalsOf(groups);
  const perAccount = byAccount(filterUsage(timeGroups, "", model));
  const colorOf = (item: string) => SERIES_COLORS[Math.max(0, perAccount.findIndex((row) => row.agent === item)) % SERIES_COLORS.length];
  const series = dailySeries(groups, days);
  const filtersActive = Boolean(agent || model);
  const lastScan = data.last_scan_at ? new Date(data.last_scan_at).toLocaleString("zh-CN", { hour12: false }) : null;

  return (
    <div className="space-y-4">
      <PageHeader
        title="用量"
        description="按账号与模型累加本机会话记录里的 token。数字来自本机文件，不联网。"
      />
      {query.data.errors.length > 0 && <ErrorPanel title="用量读取失败" errors={query.data.errors} />}

      <div className="flex flex-wrap items-center gap-2">
        <div className="inline-flex rounded-lg border bg-muted p-0.5" role="group" aria-label="时间范围">
          {RANGES.map((range) => (
            <button
              key={range.days}
              type="button"
              aria-pressed={days === range.days}
              data-testid={`usage-range-${range.days}`}
              onClick={() => {
                setDays(range.days);
                setAgent("");
                setModel("");
              }}
              className={cn(
                "rounded-md px-3 py-1.5 text-xs transition-colors",
                days === range.days ? "bg-card font-medium text-foreground shadow-sm" : "text-muted-foreground",
              )}
            >
              {range.label}
            </button>
          ))}
        </div>
        <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
          账号
          <select
            data-testid="usage-agent-filter"
            aria-label="按账号筛选用量"
            value={agent}
            onChange={(event) => {
              setAgent(event.target.value);
              setModel("");
            }}
            className="rounded-md border border-input bg-card px-2 py-1.5 text-xs text-foreground"
          >
            <option value="">全部账号</option>
            {agents.map((item) => <option key={item} value={item}>{labelOf(item)}</option>)}
          </select>
        </label>
        <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
          模型
          <select
            data-testid="usage-model-filter"
            aria-label="按模型筛选用量"
            value={model}
            onChange={(event) => setModel(event.target.value)}
            className="max-w-56 rounded-md border border-input bg-card px-2 py-1.5 font-mono text-xs text-foreground"
          >
            <option value="">全部模型</option>
            {models.map((item) => <option key={item} value={item}>{item}</option>)}
          </select>
        </label>
        {filtersActive && (
          <button
            type="button"
            data-testid="usage-clear-filters"
            onClick={() => {
              setAgent("");
              setModel("");
            }}
            className="rounded-md border px-3 py-1.5 text-xs text-muted-foreground hover:bg-accent"
          >
            清除筛选
          </button>
        )}
        <button
          type="button"
          data-testid="usage-rescan"
          disabled={scanning}
          onClick={() => void rescan()}
          className="inline-flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-xs text-muted-foreground hover:bg-accent disabled:opacity-60"
        >
          <RefreshCw className={cn("h-3.5 w-3.5", scanning && "animate-spin")} aria-hidden />
          {scanning ? "正在读…" : "重新统计"}
        </button>
        {lastScan && <span className="text-xs text-muted-foreground">上次读到 {lastScan}</span>}
      </div>

      <section className="rounded-lg border bg-card p-5 shadow-sm">
        <div className="flex flex-wrap items-baseline gap-x-6 gap-y-2">
          <div>
            <p className="text-xs text-muted-foreground">按 API 价折算</p>
            <p data-testid="usage-total-cost" className="mt-0.5 text-xl font-semibold tabular-nums text-foreground">
              {formatCost(totals.cost)}
            </p>
          </div>
          {[
            ["输入", totals.input],
            ["输出", totals.output],
            ["缓存写入", totals.cacheWrite],
            ["缓存读取", totals.cacheRead],
            ["请求数", totals.requests],
          ].map(([label, value]) => (
            <div key={label as string}>
              <p className="text-xs text-muted-foreground">{label}</p>
              <p className="mt-0.5 text-sm tabular-nums text-foreground">
                {label === "请求数" ? (value as number) : shortTokens(value as number)}
              </p>
            </div>
          ))}
          {cacheHitRate(totals) !== null && (
            <div title="缓存读取 ÷（输入 + 缓存写入 + 缓存读取）：上下文里有多少走了提示缓存">
              <p className="text-xs text-muted-foreground">缓存命中</p>
              <p data-testid="usage-cache-hit" className="mt-0.5 text-sm tabular-nums text-foreground">
                {Math.round((cacheHitRate(totals) ?? 0) * 100)}%
              </p>
            </div>
          )}
        </div>
        <p className="mt-3 text-xs text-muted-foreground">
          这是「同样的用量按 API 单价要花多少」，不是你的账单。订阅制下该付的是订阅费，
          这个数只用来看哪些模型、哪些天吃掉了用量。
        </p>
        {totals.unpricedModels.length > 0 && (
          <p data-testid="usage-unpriced" className="mt-3 text-xs text-warning">
            这些模型没有单价，没算进折算：
            {totals.unpricedModels.map((model) => (
              <button
                key={model}
                type="button"
                data-testid={`set-price-${model}`}
                onClick={() => setPricing(model)}
                className="ml-1 rounded border border-warning/40 px-1.5 py-0.5 font-mono hover:bg-warning/10"
              >
                {model}
              </button>
            ))}
          </p>
        )}
      </section>

      {groups.length > 0 && days !== 1 && (
        <UsageTrend series={series} agents={perAccount.map((row) => row.agent).filter((item) => !agent || item === agent)} colorOf={colorOf} labelOf={labelOf} />
      )}

      {perAccount.length > 1 && !agent && (
        <section data-testid="usage-by-account" className="rounded-lg border bg-card p-5 shadow-sm">
          <h3 className="mb-2 text-sm font-medium">按账号</h3>
          <table className="w-full text-left text-sm">
            <thead>
              <tr className="border-b text-xs text-muted-foreground">
                <th className="py-2 pr-4 font-medium">账号</th>
                <th className="py-2 pr-4 text-right font-medium">输入</th>
                <th className="py-2 pr-4 text-right font-medium">输出</th>
                <th className="py-2 pr-4 text-right font-medium">请求</th>
                <th className="py-2 text-right font-medium">折算</th>
              </tr>
            </thead>
            <tbody>
              {perAccount.map((row) => (
                <tr key={row.agent} data-testid={`usage-account-${row.agent}`} className="border-b last:border-0">
                  <td className="py-1.5 pr-4 text-sm">
                    <button type="button" onClick={() => setAgent(row.agent)} className="inline-flex items-center gap-1.5 hover:text-primary" title="只看这个账号">
                      <span className="h-2 w-2 rounded-sm" style={{ background: colorOf(row.agent) }} aria-hidden />
                      <AgentIcon kind={usageAccountKey(row.agent).kind} />
                      {labelOf(row.agent)}
                    </button>
                  </td>
                  <td className="py-1.5 pr-4 text-right text-xs tabular-nums">{shortTokens(row.totals.input)}</td>
                  <td className="py-1.5 pr-4 text-right text-xs tabular-nums">{shortTokens(row.totals.output)}</td>
                  <td className="py-1.5 pr-4 text-right text-xs tabular-nums text-muted-foreground">{row.totals.requests}</td>
                  <td
                    className={cn("py-1.5 text-right text-xs tabular-nums", row.totals.unpricedModels.length > 0 && "text-muted-foreground")}
                    title={row.totals.unpricedModels.length ? `没有单价、没算进去：${row.totals.unpricedModels.join("、")}` : undefined}
                  >
                    {/* 全部没有单价就不报；部分没有单价报下限，不拿 0 冒充 */}
                    {row.totals.unpricedModels.length && row.totals.cost === 0
                      ? "—"
                      : `${row.totals.unpricedModels.length ? "≥" : ""}${formatCost(row.totals.cost)}`}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}

      <RecentSessions agent={agent} labelOf={labelOf} />

      {groups.length === 0 ? (
        <EmptyState
          testId="usage-empty"
          title={filtersActive ? "当前筛选没有用量记录" : "这段时间没有用量记录"}
          description={filtersActive
            ? "换一个账号或模型，或清除筛选查看这段时间的全部记录。"
            : "换个时间范围，或点「重新统计」读一次会话记录。刚用过助手的话，它要写完这一轮才会出现在这里。"}
          actions={filtersActive ? (
            <button
              type="button"
              onClick={() => {
                setAgent("");
                setModel("");
              }}
              className="rounded-md border px-3 py-1.5 text-xs text-foreground hover:bg-accent"
            >
              清除筛选
            </button>
          ) : undefined}
        />
      ) : (
        <section className="rounded-lg border bg-card p-5 shadow-sm">
          <div className="overflow-x-auto">
            <table className="w-full min-w-[42rem] text-left text-sm">
              <thead>
                <tr className="border-b text-xs text-muted-foreground">
                  <th className="py-2 pr-4 font-medium">日期</th>
                  <th className="py-2 pr-4 font-medium">模型</th>
                  <th className="py-2 pr-4 text-right font-medium">输入</th>
                  <th className="py-2 pr-4 text-right font-medium">输出</th>
                  <th className="py-2 pr-4 text-right font-medium">缓存读</th>
                  <th className="py-2 pr-4 text-right font-medium">请求</th>
                  <th className="py-2 text-right font-medium">折算</th>
                </tr>
              </thead>
              <tbody>
                {groups.map((group) => (
                <tr key={`${group.day}/${group.agent}/${group.model}`} data-testid={`usage-row-${group.day}-${group.model}`} className="border-b last:border-0">
                  <td className="py-1.5 pr-4 text-xs text-muted-foreground">{group.day}</td>
                  <td className="py-1.5 pr-4 font-mono text-xs text-foreground">
                    {group.model}
                    <span className="ml-1.5 font-sans text-muted-foreground">{labelOf(group.agent)}</span>
                  </td>
                  <td className="py-1.5 pr-4 text-right text-xs tabular-nums">{shortTokens(group.input)}</td>
                  <td className="py-1.5 pr-4 text-right text-xs tabular-nums">{shortTokens(group.output)}</td>
                  <td className="py-1.5 pr-4 text-right text-xs tabular-nums text-muted-foreground">
                    {shortTokens(group.cacheRead)}
                  </td>
                  <td className="py-1.5 pr-4 text-right text-xs tabular-nums text-muted-foreground">
                    {group.requests}
                  </td>
                  <td className={cn("py-1.5 text-right text-xs tabular-nums", group.cost === null && "text-muted-foreground")}>
                    <button
                      type="button"
                      title="改这个模型的单价"
                      onClick={() => setPricing(group.model)}
                      className="rounded px-1 hover:bg-accent"
                    >
                      {formatCost(group.cost)}
                    </button>
                  </td>
                </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}
      {pricing && (
        <PriceDialog
          model={pricing}
          current={priceOf(pricing, data.pricing)}
          pricing={data.pricing}
          onClose={() => setPricing(null)}
          onSaved={() => void queryClient.invalidateQueries({ queryKey: engineKeys.all })}
        />
      )}
    </div>
  );
}
