import { useState } from "react";
import { Loader2, Pencil, Plus } from "lucide-react";
import { toast } from "sonner";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { Diagnostic, Envelope, PolicyData, PolicyLayer, PolicyLayerName } from "../lib/contract";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { getDefaultPolicyLayer, useDefaultPolicyLayer } from "../lib/policyLayer";
import { useEngineQuery } from "../lib/queries";
import { cn } from "../lib/utils";

/** 层归属徽标配色（提案 §4.9：baseline 灰显只读、synced 蓝标、local 橙标）。 */
const LAYER_META: Record<PolicyLayerName, { label: string; className: string }> = {
  baseline: { label: "基线层", className: "border-slate-200 bg-slate-100 text-slate-500" },
  synced: { label: "共享层", className: "border-sky-200 bg-sky-50 text-sky-700" },
  local: { label: "仅本机", className: "border-orange-200 bg-orange-50 text-orange-700" },
};

function LayerBadge({ layer }: { layer: PolicyLayerName }) {
  const meta = LAYER_META[layer];
  return (
    <span data-testid={`layer-badge-${layer}`} className={cn("rounded border px-1.5 py-0.5 text-xs whitespace-nowrap", meta.className)}>
      {meta.label}
    </span>
  );
}

interface FlatRow {
  path: string;
  value: unknown;
}

/** 把 effective 树摊平成叶子行（点分路径），深度防爆。 */
function flatten(value: unknown, prefix: string, depth: number, out: FlatRow[]): void {
  if (depth > 8) return;
  if (value !== null && typeof value === "object" && !Array.isArray(value)) {
    const entries = Object.entries(value as Record<string, unknown>);
    if (entries.length === 0) {
      out.push({ path: prefix, value });
      return;
    }
    for (const [key, child] of entries) {
      flatten(child, prefix ? `${prefix}.${key}` : key, depth + 1, out);
    }
    return;
  }
  out.push({ path: prefix, value });
}

function shortValue(value: unknown): string {
  const rendered = JSON.stringify(value);
  if (rendered === undefined) return "undefined";
  return rendered.length > 60 ? `${rendered.slice(0, 57)}…` : rendered;
}

/** 顶层键的层归属 = attribution；更深层沿顶层归属展示（引擎只提供顶层归属）。 */
function layerOfTopKey(attribution: Record<string, PolicyLayerName> | null, path: string): PolicyLayerName {
  const top = path.split(".")[0];
  return attribution?.[top] ?? "baseline";
}

function EditDialog({
  initialKey,
  initialValue,
  onClose,
  onSaved,
}: {
  initialKey: string;
  initialValue: unknown;
  onClose: () => void;
  onSaved: (data: PolicyData) => void;
}) {
  const [key, setKey] = useState(initialKey);
  const [rawValue, setRawValue] = useState(() => (initialValue === undefined ? "" : JSON.stringify(initialValue, null, 2)));
  const [layer, setLayer] = useState<PolicyLayer>(getDefaultPolicyLayer());
  const [submitting, setSubmitting] = useState(false);
  const [errors, setErrors] = useState<Diagnostic[] | null>(null);
  const [clientError, setClientError] = useState<string | null>(null);

  const submit = async () => {
    setClientError(null);
    setErrors(null);
    if (!/^[A-Za-z0-9_-]+(\.[A-Za-z0-9_-]+)*$/.test(key)) {
      setClientError("非法 key：点号分隔，每段只允许字母/数字/_/-");
      return;
    }
    try {
      JSON.parse(rawValue);
    } catch (err) {
      setClientError(`值不是合法 JSON：${err instanceof Error ? err.message : String(err)}`);
      return;
    }
    setSubmitting(true);
    let envelope: Envelope<PolicyData>;
    try {
      envelope = await runEngine<PolicyData>("policy:set", ["--layer", layer, "--key", key, "--value", rawValue]);
    } catch (err) {
      setSubmitting(false);
      setErrors([{ code: "BRIDGE", element: null, message: engineErrorMessage(err) }]);
      return;
    }
    setSubmitting(false);
    if (!envelope.ok) {
      // 引擎门禁：非法 JSON / 疑似明文（POLICY_SECRET_GATE）等
      setErrors(envelope.errors);
      return;
    }
    toast.success(`已写入 ${LAYER_META[layer].label}：${key}`);
    onSaved(envelope.data);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div role="dialog" aria-modal="true" aria-label="编辑策略键" className="w-full max-w-xl rounded-xl border border-slate-200 bg-white p-5 shadow-xl">
        <h3 className="text-base font-semibold text-slate-800">编辑策略键</h3>
        <div className="mt-4 space-y-3">
          <div>
            <label className="mb-1 block text-xs font-medium text-slate-500">键（点分路径）</label>
            <input
              type="text"
              value={key}
              onChange={(event) => setKey(event.target.value)}
              data-testid="policy-key-input"
              className="w-full rounded border border-slate-300 px-2 py-1.5 font-mono text-xs"
            />
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium text-slate-500">值（JSON）</label>
            <textarea
              value={rawValue}
              onChange={(event) => setRawValue(event.target.value)}
              rows={5}
              data-testid="policy-value-input"
              className="w-full rounded border border-slate-300 px-2 py-1.5 font-mono text-xs"
            />
          </div>
          <fieldset>
            <legend className="mb-1 text-xs font-medium text-slate-500">落点（baseline 基线层只读，改动走正常 git 流程）</legend>
            <div className="flex gap-3">
              {(["synced", "local"] as const).map((candidate) => (
                <label key={candidate} className="flex items-center gap-1.5 text-xs text-slate-700">
                  <input
                    type="radio"
                    name="policy-layer"
                    checked={layer === candidate}
                    onChange={() => setLayer(candidate)}
                    data-testid={`policy-layer-${candidate}`}
                  />
                  <LayerBadge layer={candidate} />
                  {candidate === "synced" ? "（随配置同步走）" : "（只影响本机）"}
                </label>
              ))}
            </div>
          </fieldset>
          {clientError && <p className="text-xs text-red-600">{clientError}</p>}
          {errors && errors.length > 0 && <ErrorPanel title="policy:set 被引擎拒绝" errors={errors} />}
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <button type="button" onClick={onClose} disabled={submitting} className="rounded-md border border-slate-200 px-4 py-2 text-sm text-slate-600 hover:bg-slate-50 disabled:opacity-40">
            取消
          </button>
          <button
            type="button"
            onClick={() => void submit()}
            disabled={submitting}
            data-testid="policy-set-submit"
            className={cn(
              "inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white",
              submitting ? "cursor-not-allowed bg-slate-300" : "bg-indigo-600 hover:bg-indigo-700",
            )}
          >
            {submitting && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
            写入
          </button>
        </div>
      </div>
    </div>
  );
}

// 模块内读取默认层（避免 hook 顺序约束；dialog 打开时取当前值）

export default function PolicyPage() {
  const query = useEngineQuery<PolicyData>("policy:get");
  const [defaultLayer, setDefaultLayer] = useDefaultPolicyLayer();
  const [editing, setEditing] = useState<{ key: string; value: unknown } | null>(null);

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="policy:get 调用失败" error={query.error} />;

  const data = query.data.data;
  const attribution = data.attribution;
  const rows: FlatRow[] = [];
  if (data.effective) flatten(data.effective, "", 0, rows);

  return (
    <div className="space-y-4">
      <PageHeader
        title="策略编辑器"
        description="三层栈 manager.json（基线）⊕ manager.synced.json（共享）⊕ manager.local.json（仅本机）。写入只落 synced/local 层，疑似明文有引擎硬门禁。"
      />
      {query.data.errors.length > 0 && <ErrorPanel title="policy:get 错误" errors={query.data.errors} />}

      <section className="flex flex-wrap items-center gap-3 rounded-lg border border-slate-200 bg-white p-4 shadow-sm">
        <span className="text-xs text-slate-500">快捷写路径（如 Skills 矩阵点击）的默认落点：</span>
        {(["local", "synced"] as const).map((candidate) => (
          <label key={candidate} className="flex items-center gap-1.5 text-xs text-slate-700">
            <input
              type="radio"
              name="default-policy-layer"
              checked={defaultLayer === candidate}
              onChange={() => setDefaultLayer(candidate)}
              data-testid={`default-layer-${candidate}`}
            />
            <LayerBadge layer={candidate} />
          </label>
        ))}
        <button
          type="button"
          onClick={() => setEditing({ key: "", value: undefined })}
          className="ml-auto inline-flex items-center gap-1 rounded-md border border-indigo-200 px-3 py-1.5 text-xs text-indigo-600 hover:bg-indigo-50"
        >
          <Plus className="h-3.5 w-3.5" aria-hidden /> 新建键
        </button>
      </section>

      <section className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <h3 className="mb-3 text-sm font-semibold text-slate-700">生效配置（合并视图）</h3>
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-slate-200 text-xs text-slate-500">
              <th className="py-2 pr-4 font-medium">键</th>
              <th className="py-2 pr-4 font-medium">值（摘要）</th>
              <th className="py-2 pr-4 font-medium">层归属</th>
              <th className="py-2 font-medium">操作</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.path} className="border-b border-slate-100 last:border-0">
                <td className="py-1.5 pr-4 font-mono text-xs break-all text-slate-700">{row.path}</td>
                <td className="py-1.5 pr-4 font-mono text-xs text-slate-500">{shortValue(row.value)}</td>
                <td className="py-1.5 pr-4">
                  <LayerBadge layer={layerOfTopKey(attribution, row.path)} />
                </td>
                <td className="py-1.5">
                  <button
                    type="button"
                    onClick={() => setEditing({ key: row.path, value: row.value })}
                    data-testid={`edit-${row.path}`}
                    className="inline-flex items-center gap-1 rounded border border-slate-200 px-2 py-0.5 text-xs text-slate-600 hover:bg-slate-50"
                  >
                    <Pencil className="h-3 w-3" aria-hidden /> 编辑
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length === 0 && <p className="py-4 text-center text-sm text-slate-400">配置为空</p>}
      </section>

      <section className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        {(["baseline", "synced", "local"] as const).map((layer) => (
          <div key={layer} data-testid={`layer-card-${layer}`} className="rounded-lg border border-slate-200 bg-white p-4 shadow-sm">
            <div className="mb-2 flex items-center justify-between">
              <LayerBadge layer={layer} />
              {layer === "baseline" && <span className="text-xs text-slate-400">只读</span>}
            </div>
            <pre className="max-h-64 overflow-auto rounded bg-slate-50 p-2 font-mono text-xs text-slate-600">
              {data.layers?.[layer] ? JSON.stringify(data.layers[layer], null, 2) : "（文件不存在）"}
            </pre>
          </div>
        ))}
      </section>

      {editing && (
        <EditDialog
          initialKey={editing.key}
          initialValue={editing.value}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            void query.refetch();
          }}
        />
      )}
    </div>
  );
}
