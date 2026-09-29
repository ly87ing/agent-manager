import { useState } from "react";
import { toast } from "sonner";
import { Download, Upload } from "lucide-react";
import { diffBackup, sourceLabel, type BackupDiff } from "../lib/backupDiff";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { engineKeys } from "../lib/queries";
import { useQueryClient } from "@tanstack/react-query";
import { useDialogFocus } from "../lib/dialogFocus";

/**
 * 配置备份：把本机的定义与来源导出成一份 JSON，换机器或重装时导回来。
 *
 * 密钥取值与外部文件路径不进入备份。不含分配记录与写入指纹——那些是本机落地状态，
 * 导回来之后仍由用户到对应页面逐项预览和应用，才会写到助手配置里。
 */
export function ConfigBackupSection() {
  const queryClient = useQueryClient();
  const [exported, setExported] = useState<string | null>(null);
  /** 导出内容里有什么：只报条数，正文不铺在页面上。 */
  const [exportSummary, setExportSummary] = useState<string | null>(null);
  const [importText, setImportText] = useState("");
  const [showImport, setShowImport] = useState(false);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [imported, setImported] = useState(false);
  /** 确认前算好的差异：导入不可撤销，先让人看清会改掉什么。 */
  const [diff, setDiff] = useState<BackupDiff | null>(null);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({
    active: confirming && diff !== null,
    onEscape: busy ? undefined : () => setConfirming(false),
    initialFocusSelector: '[data-testid="config-import-cancel"]',
  });

  const doExport = async () => {
    setBusy(true);
    try {
      const envelope = await runEngine<Record<string, unknown>>("config:export");
      if (!envelope.ok) {
        toast.error(`导出失败:${envelope.errors[0]?.message ?? "未知错误"}`);
        return;
      }
      const data = envelope.data as {
        policy?: Record<string, unknown>;
        sources?: Record<string, unknown>;
        rule_templates?: unknown[];
        mcp_saved?: unknown[];
      };
      const policy = (data.policy ?? {}) as Record<string, any>;
      const count = (obj: unknown) => Object.keys((obj ?? {}) as Record<string, unknown>).length;
      const definitions = count(policy.agents?.definitions) + count(policy.skills?.repositories);
      const mcpSaved = data.mcp_saved?.length ?? 0;
      setExported(JSON.stringify(envelope.data, null, 2));
      setExportSummary(
        `${definitions} 条定义${mcpSaved > 0 ? ` · ${mcpSaved} 个 MCP 已保存项` : ""} · ${count(data.sources)} 份来源 · ${data.rule_templates?.length ?? 0} 个规则模板 · 不含密钥`,
      );
      toast.success("已生成备份内容");
    } catch (err) {
      toast.error(`导出失败:${engineErrorMessage(err)}`);
    } finally {
      setBusy(false);
    }
  };

  const copy = async () => {
    if (!exported) return;
    try {
      await navigator.clipboard.writeText(exported);
      toast.success("已复制到剪贴板");
    } catch {
      toast.error("复制失败，请手动选中文本复制");
    }
  };

  /** 点「导入并覆盖」先算差异：读当前配置与粘贴内容对比，算不出来就不让往下走。 */
  const prepareImport = async () => {
    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(importText.trim());
    } catch {
      toast.error("粘贴的内容不是合法 JSON");
      return;
    }
    const legacySecrets = parsed.secrets;
    const isEmptyLegacySecrets =
      legacySecrets === undefined ||
      legacySecrets === null ||
      (typeof legacySecrets === "object" &&
        !Array.isArray(legacySecrets) &&
        Object.keys(legacySecrets).length === 0);
    if (!isEmptyLegacySecrets) {
      toast.error("这份旧备份含明文凭据，已停止导入", {
        description: "请先把凭据移到外部密钥文件，并从备份中删除 secrets 段。",
      });
      return;
    }
    setBusy(true);
    try {
      const envelope = await runEngine<Record<string, unknown>>("config:export");
      if (!envelope.ok) {
        toast.error(`读取当前配置失败:${envelope.errors[0]?.message ?? "未知错误"}`);
        return;
      }
      setDiff(diffBackup(envelope.data, parsed));
      setConfirming(true);
    } catch (err) {
      toast.error(`读取当前配置失败:${engineErrorMessage(err)}`);
    } finally {
      setBusy(false);
    }
  };

  const doImport = async () => {
    setConfirming(false);
    setBusy(true);
    try {
      const envelope = await runEngine("config:import", ["--value", importText.trim()]);
      if (!envelope.ok) {
        toast.error(`导入失败:${envelope.errors[0]?.message ?? "未知错误"}`);
        return;
      }
      setImportText("");
      setImported(true);
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
      toast.success("已恢复定义与来源", {
        description: "助手配置未改动；请到对应页面逐项预览和应用，密钥需另行配置。",
      });
    } catch (err) {
      toast.error(`导入失败:${engineErrorMessage(err)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <section data-testid="config-backup-section" className="overflow-clip rounded-lg border bg-card" aria-label="配置备份">
      <h2 className="border-b bg-muted px-4 py-2 text-xs font-medium text-foreground">配置备份</h2>
      <div className="divide-y">
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3">
          <div className="min-w-0 flex-1">
            <div className="text-sm">导出配置</div>
            <div className="text-xs text-muted-foreground">助手、技能、MCP、账号定义与规则来源；不含密钥取值与外部文件路径</div>
          </div>
          <button
            type="button"
            data-testid="config-export-button"
            disabled={busy}
            onClick={() => void doExport()}
            className="inline-flex items-center gap-1.5 rounded-md border px-3 py-1 text-sm hover:bg-accent disabled:opacity-50"
          >
            <Download className="h-3.5 w-3.5" aria-hidden /> 导出
          </button>
          {exported && (
            <div className="flex w-full flex-wrap items-center gap-2 text-xs">
              <p data-testid="config-export-output" className="min-w-0 flex-1 text-foreground">
                已生成：{exportSummary}。已确认不含密钥取值，也不含外部密钥文件路径；换机器后需要重新选择那台机器自己的密钥文件。
              </p>
              <button
                type="button"
                data-testid="config-copy-button"
                onClick={() => void copy()}
                className="rounded-md border px-2.5 py-1 text-xs hover:bg-accent"
              >
                复制到剪贴板
              </button>
            </div>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3">
          <div className="min-w-0 flex-1">
            <div className="text-sm">从备份恢复</div>
            <div className="text-xs text-muted-foreground">只恢复定义与来源，不改助手配置；导回后到各页逐项预览和应用</div>
          </div>
          <button
            type="button"
            data-testid="config-import-toggle"
            aria-expanded={showImport}
            onClick={() => setShowImport((value) => !value)}
            className="inline-flex items-center gap-1.5 rounded-md border px-3 py-1 text-sm hover:bg-accent"
          >
            <Upload className="h-3.5 w-3.5" aria-hidden /> {showImport ? "收起" : "粘贴备份…"}
          </button>
          {showImport && (
            <div className="w-full space-y-1.5">
              <label className="sr-only" htmlFor="config-import-input">
                粘贴备份内容以恢复
              </label>
              <textarea
                id="config-import-input"
                value={importText}
                onChange={(event) => {
                  setImportText(event.target.value);
                  setImported(false);
                }}
                rows={4}
                spellCheck={false}
                data-testid="config-import-input"
                placeholder='粘贴导出的备份内容：{"format":"agent-manager/config-backup/v1", …}'
                className="w-full rounded-md border border-input bg-background px-2 py-1.5 font-mono text-[11px]"
              />
              <button
                type="button"
                data-testid="config-import-button"
                disabled={busy || !importText.trim()}
                onClick={() => void prepareImport()}
                className="inline-flex items-center gap-1 rounded-md border border-warning/40 bg-warning/10 px-2.5 py-1 text-xs text-warning hover:bg-warning/15 disabled:opacity-50"
              >
                导入并覆盖…
              </button>
            </div>
          )}
          {imported && (
            <p data-testid="config-import-success" className="w-full text-xs text-success">
              定义与来源已恢复，助手配置未改动。请到账号、技能、MCP、规则等对应页面逐项预览和应用；密钥需另行配置。
            </p>
          )}
        </div>
      </div>

      {confirming && diff && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in"
          onClick={(event) => event.target === event.currentTarget && setConfirming(false)}
        >
          <div
            ref={dialogRef}
            role="dialog"
            aria-modal="true"
            aria-label="导入前的改动预览"
            data-testid="config-import-diff"
            tabIndex={-1}
            onKeyDown={onDialogKeyDown}
            className="max-h-[85vh] w-full max-w-lg overflow-y-auto rounded-xl border bg-card p-5 shadow-xl animate-zoom-in-95"
          >
            <h3 className="text-base font-semibold text-foreground">这份备份会改掉这些</h3>
            <p className="mt-1 text-xs text-muted-foreground">
              导入是整体替换，不能撤销。助手配置文件本身不受影响；恢复后到对应页面逐项预览和应用。
            </p>

            <div className="mt-3 space-y-2 text-sm">
              {diff.identical ? (
                <p data-testid="config-diff-identical" className="rounded-md border bg-muted px-3 py-2 text-xs text-muted-foreground">
                  与当前配置完全一致，导入不会改变任何东西。
                </p>
              ) : (
                <>
                  {diff.sections.map((section) => (
                    <div key={section.label} data-testid={`config-diff-${section.label}`} className="rounded-md border px-3 py-2">
                      <p className="text-xs font-medium text-foreground">{section.label}</p>
                      <ul className="mt-1 space-y-0.5 text-[11px]">
                        {section.added.length > 0 && (
                          <li className="text-success">新增：{section.added.join("、")}</li>
                        )}
                        {section.changed.length > 0 && (
                          <li className="text-warning">改动：{section.changed.join("、")}</li>
                        )}
                        {section.removed.length > 0 && (
                          <li className="text-destructive">消失：{section.removed.join("、")}</li>
                        )}
                      </ul>
                    </div>
                  ))}
                  {diff.sourcesChanged.length > 0 && (
                    <div data-testid="config-diff-sources" className="rounded-md border px-3 py-2">
                      <p className="text-xs font-medium text-foreground">来源内容</p>
                      <p className="mt-1 text-[11px] text-warning">
                        会被替换：{diff.sourcesChanged.map(sourceLabel).join("、")}
                      </p>
                    </div>
                  )}
                </>
              )}
            </div>

            <div className="mt-5 flex items-center justify-end gap-2">
              <button
                type="button"
                data-testid="config-import-cancel"
                onClick={() => setConfirming(false)}
                className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent"
              >
                取消
              </button>
              <button
                type="button"
                data-testid="config-import-confirm"
                disabled={busy}
                onClick={() => void doImport()}
                className="rounded-md border border-warning/40 bg-warning/10 px-4 py-2 text-sm font-medium text-warning hover:bg-warning/15 disabled:opacity-50"
              >
                确认覆盖
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
