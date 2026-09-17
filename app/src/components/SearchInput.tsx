import { Search } from "lucide-react";

/**
 * 页内搜索输入框：四个列表页共用（data-page-search 是全局快捷键 ⌘F 的聚焦锚点）。
 * 过滤为受控值驱动的大小写不敏感匹配，中文输入法组合期间的中间串照常参与过滤，不打断组合。
 */
export function SearchInput({
  value,
  onChange,
  placeholder,
  testId,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  testId: string;
}) {
  return (
    <div className="relative">
      <Search
        className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground"
        aria-hidden
      />
      <input
        type="search"
        data-page-search
        data-testid={testId}
        aria-label={placeholder}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        className="w-full rounded-md border border-input bg-card py-1.5 pl-8 pr-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-ring sm:w-64"
      />
    </div>
  );
}

/** 大小写不敏感的关键词匹配：keyword 为空恒 true；任一字段包含即命中。 */
export function matchesKeyword(keyword: string, fields: (string | null | undefined)[]): boolean {
  const needle = keyword.trim().toLowerCase();
  if (!needle) return true;
  return fields.some((field) => field?.toLowerCase().includes(needle));
}
