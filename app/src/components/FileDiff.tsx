import { t } from "../lib/i18n";
import { useMemo, useState } from "react";
import { ChevronRight } from "lucide-react";
import { diffLines, diffStats, foldDiff, type DiffSegment } from "../lib/textDiff";
import { cn } from "../lib/utils";

/**
 * 写入前的文件变化：只显示改动的行和前后各 3 行，没变的折叠成「⋯ 未改动 N 行」（点一下展开），
 * 标题写出「新建 / 更新」与「+新增 −删除」。整份文件收在「查看完整文件」里，需要时再看。
 * 所有「写之前让用户确认」的预览共用这一个（MCP、快捷命令、新建账号、规则）。
 */
export function FileDiff({
  path,
  original,
  content,
  testId,
  maxHeight = "max-h-72",
}: {
  path?: string;
  /** 写入前的原文；文件不存在为 null。 */
  original: string | null | undefined;
  content: string;
  testId?: string;
  maxHeight?: string;
}) {
  const [expanded, setExpanded] = useState<Set<number>>(() => new Set());
  const [showFull, setShowFull] = useState(false);
  const isNew = original === null || original === undefined;
  const lines = useMemo(() => diffLines(original ?? "", content), [original, content]);
  const stats = diffStats(lines);
  const segments: DiffSegment[] = useMemo(() => foldDiff(lines), [lines]);
  const unchanged = !isNew && original === content;

  return (
    <div data-testid={testId} className="space-y-1 text-xs">
      <p className="flex flex-wrap items-center gap-x-2 text-muted-foreground">
        {path && <span className="break-all font-mono">{path}</span>}
        <span>{isNew ? t("新建") : unchanged ? t("内容不变") : t("更新")}</span>
        {!unchanged && (
          <span data-testid={testId ? `${testId}-stats` : undefined} className="font-mono">
            <span className="text-success">+{stats.added}</span> <span className="text-destructive">−{stats.removed}</span>
          </span>
        )}
      </p>
      {!unchanged && (
        <div className={cn("overflow-auto rounded border bg-muted/40 font-mono text-[11px]", maxHeight)}>
          {segments.map((segment, index) =>
            segment.type === "fold" && !expanded.has(index) ? (
              <button
                key={index}
                type="button"
                data-testid={testId ? `${testId}-fold` : undefined}
                onClick={() => setExpanded((prev) => new Set(prev).add(index))}
                className="block w-full bg-muted/60 px-2 py-0.5 text-left text-muted-foreground hover:bg-accent"
              >{(t("⋯ 未改动") + " ")}{segment.lines.length}{(" " + t("行（点击展开）"))}</button>
            ) : (
              segment.lines.map((line, lineIndex) => (
                <div
                  key={`${index}-${lineIndex}`}
                  data-kind={line.kind}
                  className={cn(
                    "flex whitespace-pre-wrap break-all",
                    line.kind === "add" && "bg-success/10 text-success",
                    line.kind === "del" && "bg-destructive/10 text-destructive",
                  )}
                >
                  <span className="w-10 shrink-0 select-none pr-2 text-right text-muted-foreground/60">{line.newNo ?? line.oldNo}</span>
                  <span className="w-3 shrink-0 select-none">{line.kind === "add" ? "+" : line.kind === "del" ? "−" : " "}</span>
                  <span className="min-w-0 flex-1">{line.text}</span>
                </div>
              ))
            ),
          )}
        </div>
      )}
      <button
        type="button"
        aria-expanded={showFull}
        data-testid={testId ? `${testId}-full-toggle` : undefined}
        onClick={() => setShowFull((value) => !value)}
        className="inline-flex items-center gap-0.5 text-muted-foreground hover:text-foreground"
      >
        <ChevronRight className={cn("h-3 w-3 transition-transform", showFull && "rotate-90")} aria-hidden />
        {showFull ? t("收起完整文件") : t("查看完整文件")}
      </button>
      {showFull && (
        <pre
          data-testid={testId ? `${testId}-full` : undefined}
          className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono text-[11px]"
        >
          {content}
        </pre>
      )}
    </div>
  );
}

/** 一处文件变化：改前 / 改后原文（contents 为 false 时太大、二进制或超出数量，只给文件名）。 */
export interface FileChange {
  status: string;
  file: string;
  original?: string | null;
  content?: string | null;
  contents?: boolean;
}

/** 文件变化清单里的一行：点开只显示这个文件改动的行（与 FileDiff 同一写法）。 */
export function ChangedFile({ change, label, testId }: { change: FileChange; label: string; testId?: string }) {
  const [open, setOpen] = useState(false);
  const viewable = change.contents === true;
  return (
    <li data-testid={testId} className="font-mono">
      <button
        type="button"
        disabled={!viewable}
        aria-expanded={viewable ? open : undefined}
        onClick={() => setOpen((value) => !value)}
        className={cn("inline-flex items-center gap-1 text-left", viewable ? "hover:text-foreground" : "cursor-default")}
        title={viewable ? t("点开看改了什么") : t("太大、二进制或文件太多，未显示内容")}
      >
        {viewable ? (
          <ChevronRight className={cn("h-3 w-3 shrink-0 transition-transform", open && "rotate-90")} aria-hidden />
        ) : (
          <span className="w-3" />
        )}
        <span className="text-muted-foreground">{label}</span> {change.file}
        {!viewable && <span className="font-sans text-[10px] text-muted-foreground">{t("（未显示内容）")}</span>}
      </button>
      {open && viewable && (
        <div className="mt-1 font-sans">
          <FileDiff original={change.original ?? null} content={change.content ?? ""} maxHeight="max-h-60" />
        </div>
      )}
    </li>
  );
}
