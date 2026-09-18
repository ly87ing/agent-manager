import { useState } from "react";
import { toast } from "sonner";
import { Download, Upload } from "lucide-react";
import { ConfirmModal } from "./ConfirmModal";
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
            onClick={() => setConfirming(true)}
            className="mt-1.5 inline-flex items-center gap-1 rounded border border-warning/40 bg-warning/10 px-2.5 py-1 text-xs text-warning hover:bg-warning/15 disabled:opacity-50"
          >
            <Upload className="h-3.5 w-3.5" aria-hidden /> 导入并覆盖
          </button>
        </div>
      </div>

      {confirming && (
        <ConfirmModal
          title="用备份覆盖本机配置"
          description="本机现有的助手、技能、MCP、实例定义与规则等来源会被这份备份整体替换，这一步不能撤销。助手配置文件不受影响——恢复后要到各页面点一次应用才会落地。"
          risk="write-local"
          confirmLabel="确认覆盖"
          executing={busy}
          onConfirm={() => void doImport()}
          onClose={() => setConfirming(false)}
        />
      )}
    </section>
  );
}
