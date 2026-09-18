import { useState } from "react";
import { toast } from "sonner";
import { Download, Upload } from "lucide-react";
import { diffBackup, sourceLabel, type BackupDiff } from "../lib/backupDiff";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { engineKeys } from "../lib/queries";
import { useQueryClient } from "@tanstack/react-query";

/**
 * 配置备份：把本机的定义与来源导出成一份 JSON，换机器或重装时导回来。
 *
 * 备份里只有定义与来源，没有分配记录与写入指纹——那些是本机落地状态，
 * 导回来之后还要各自应用一次才会写到助手配置里。凭据本来就不入库，
 * 备份里只有 ${变量名} 引用，换机器后要自己把变量配上。
 */
export function ConfigBackupSection() {
  const queryClient = useQueryClient();
  const [exported, setExported] = useState<string | null>(null);
  const [importText, setImportText] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  /** 确认前算好的差异：导入不可撤销，先让人看清会改掉什么。 */
  const [diff, setDiff] = useState<BackupDiff | null>(null);

  const doExport = async () => {
    setBusy(true);
    try {
      const envelope = await runEngine<Record<string, unknown>>("config:export");
      if (!envelope.ok) {
        toast.error(`导出失败:${envelope.errors[0]?.message ?? "未知错误"}`);
        return;
      }
      setExported(JSON.stringify(envelope.data, null, 2));
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
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
      toast.success("已恢复定义与来源", {
        description: "还没写到助手配置：到各页面点一次应用才会落地。",
      });
    } catch (err) {
      toast.error(`导入失败:${engineErrorMessage(err)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <section data-testid="config-backup-section" className="rounded-lg border bg-card p-5 shadow-sm">
      <h3 className="text-sm font-semibold text-foreground">配置备份</h3>
      <p className="mt-1 text-xs text-muted-foreground">
        导出本机的助手、技能、MCP、实例定义与规则等来源，换机器或重装时导回来。备份里没有密钥（只有变量名），
        也不含本机的落地记录——导回来之后要到各页面点一次应用才会写到助手配置。
      </p>

      <div className="mt-3 space-y-3">
        <div>
          <button
            type="button"
            data-testid="config-export-button"
            disabled={busy}
            onClick={() => void doExport()}
            className="inline-flex items-center gap-1 rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15 disabled:opacity-50"
          >
            <Download className="h-3.5 w-3.5" aria-hidden /> 导出配置
          </button>
          {exported && (
            <div className="mt-2 space-y-1.5">
              <textarea
                readOnly
                value={exported}
                rows={6}
                data-testid="config-export-output"
                aria-label="备份内容"
                className="w-full rounded border border-input px-2 py-1.5 font-mono text-[11px]"
              />
              <button
                type="button"
                data-testid="config-copy-button"
                onClick={() => void copy()}
                className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent"
              >
                复制到剪贴板
              </button>
            </div>
          )}
        </div>

        <div className="border-t pt-3">
          <label className="mb-1 block text-xs font-medium text-muted-foreground" htmlFor="config-import-input">
            粘贴备份内容以恢复
          </label>
          <textarea
            id="config-import-input"
            value={importText}
            onChange={(event) => setImportText(event.target.value)}
            rows={4}
            spellCheck={false}
            data-testid="config-import-input"
            placeholder='{"format":"agent-manager/config-backup/v1", …}'
            className="w-full rounded border border-input px-2 py-1.5 font-mono text-[11px]"
          />
          <button
            type="button"
            data-testid="config-import-button"
            disabled={busy || !importText.trim()}
            onClick={() => void prepareImport()}
            className="mt-1.5 inline-flex items-center gap-1 rounded border border-warning/40 bg-warning/10 px-2.5 py-1 text-xs text-warning hover:bg-warning/15 disabled:opacity-50"
          >
            <Upload className="h-3.5 w-3.5" aria-hidden /> 导入并覆盖
          </button>
        </div>
      </div>

      {confirming && diff && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in"
          onClick={(event) => event.target === event.currentTarget && setConfirming(false)}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-label="导入前的改动预览"
            data-testid="config-import-diff"
            className="max-h-[85vh] w-full max-w-lg overflow-y-auto rounded-xl border bg-card p-5 shadow-xl animate-zoom-in-95"
          >
            <h3 className="text-base font-semibold text-foreground">这份备份会改掉这些</h3>
            <p className="mt-1 text-xs text-muted-foreground">
              导入是整体替换，不能撤销。助手配置文件本身不受影响——恢复后要到各页面点一次应用才会落地。
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
