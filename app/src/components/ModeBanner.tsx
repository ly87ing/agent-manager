import { FlaskConical, WifiOff } from "lucide-react";
import { useEngineQuery } from "../lib/queries";
import type { AuditData } from "../lib/contract";
import { COPY } from "../lib/ui-copy";

/**
 * 全局模式横幅：离线 / 演练（dry-run）时给出醒目标注。
 * 复用 system:audit 的 react-query 缓存，不额外发请求。
 */
export function ModeBanner() {
  const audit = useEngineQuery<AuditData>("system:audit");
  const mode = audit.data?.mode;
  if (!mode || (!mode.offline && !mode.dry_run)) {
    return null;
  }
  return (
    <div className="space-y-2">
      {mode.offline && (
        <div
          role="status"
          className="flex items-center gap-2 rounded-lg border border-amber-200 bg-amber-50 px-4 py-2 text-sm text-amber-800"
        >
          <WifiOff className="h-4 w-4 shrink-0" aria-hidden />
          <span>
            <strong>离线模式</strong>：{COPY.offlineBannerBody}
          </span>
        </div>
      )}
      {mode.dry_run && (
        <div
          role="status"
          className="flex items-center gap-2 rounded-lg border border-sky-200 bg-sky-50 px-4 py-2 text-sm text-sky-800"
        >
          <FlaskConical className="h-4 w-4 shrink-0" aria-hidden />
          <span>
            <strong>演练模式</strong>：{COPY.dryRunBannerBody}
          </span>
        </div>
      )}
    </div>
  );
}
