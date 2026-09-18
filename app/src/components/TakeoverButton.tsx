import type { EngineElement } from "../lib/contract";

/**
 * 目标被本工具以外改过时的「接管原内容」入口。
 *
 * 没有这个入口，用户手改过文件之后同步会永久失败，界面上无路可走——这是 rules 与
 * aliases 此前的实际状况。只有引擎给出了可安全接管的 actual_digest（string）时才出现：
 * 为 null 表示路径不安全、内容读不了，那种情况不提供接管。
 */

/** 接管会改掉什么，按域说清楚——用户点之前要知道代价。 */
const TAKEOVER_NOTE: Record<string, string> = {
  hooks: "整文件替换：现有内容将被整体替换，先把需要保留的 hook 并入来源定义。",
  rules: "整文件替换：这个文件里本工具以外写的内容会被整体替换，需要保留的先并进规则正文。",
  aliases: "只替换本工具维护的那一段：这段里被改过的内容会按当前配置重写，段外的内容不动。",
};

export function isTakeoverable(element: EngineElement): boolean {
  const observed = element.observed as { blocked_reason?: unknown; actual_digest?: unknown };
  if (typeof observed.actual_digest !== "string") return false;
  if (element.state === "unmanaged-detected") return true;
  return observed.blocked_reason != null;
}

export function TakeoverButton({
  element,
  domain,
  disabled,
  onTakeover,
  showNote = true,
}: {
  element: EngineElement;
  domain: string;
  disabled?: boolean;
  onTakeover: () => void;
  /** 表格行里空间紧，说明文字可以收起来只留按钮。 */
  showNote?: boolean;
}) {
  if (!isTakeoverable(element)) return null;
  const note = TAKEOVER_NOTE[domain];
  return (
    <div className="flex max-w-xs flex-col items-end gap-1 text-right">
      <button
        type="button"
        data-testid={`takeover-${element.id}`}
        disabled={disabled}
        title={note}
        onClick={onTakeover}
        className="inline-flex items-center gap-1 rounded border border-warning/40 bg-warning/10 px-2.5 py-1 text-xs font-medium text-warning hover:bg-warning/15 disabled:opacity-40"
      >
        接管原内容
      </button>
      {showNote && note && (
        <p data-testid={`takeover-hint-${element.id}`} className="text-[11px] leading-snug text-warning">
          {note}
        </p>
      )}
    </div>
  );
}
