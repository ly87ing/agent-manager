import { WifiOff } from "lucide-react";
import { useEngineQuery } from "../lib/queries";
import type { AuditData } from "../lib/contract";
import { COPY } from "../lib/ui-copy";

/**
 * 全局模式横幅：离线模式时给出醒目标注。
 * 复用 system:audit 的 react-query 缓存，不额外发请求。
 */
export function ModeBanner() {
  const audit = useEngineQuery<AuditData>("system:audit");
  const mode = audit.data?.mode;
  if (!mode?.offline) {
    return null;
  }
  return (
    <div
      role="status"
      className="flex items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 px-4 py-2 text-sm text-warning"
    >
      <WifiOff className="h-4 w-4 shrink-0" aria-hidden />
      <span>
        <strong>离线模式</strong>：{COPY.offlineBannerBody}
      </span>
    </div>
  );
}
