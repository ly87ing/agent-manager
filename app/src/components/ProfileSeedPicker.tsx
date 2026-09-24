import type { ProfileKind } from "../lib/profileDefinition";
import { SEED_ITEMS, type SeedItem } from "../lib/profileSeed";

/** 勾选要从默认环境带进独立环境的设置；凭据不在可选项里。 */
export function ProfileSeedPicker({
  kind,
  selected,
  onChange,
  testIdPrefix,
}: {
  kind: ProfileKind;
  selected: SeedItem[];
  onChange: (items: SeedItem[]) => void;
  testIdPrefix: string;
}) {
  return (
    <div className="space-y-2" data-testid={`${testIdPrefix}-items`}>
      {SEED_ITEMS.map((item) => (
        <label key={item.id} className="flex items-start gap-2 text-sm text-foreground">
          <input
            type="checkbox"
            data-testid={`${testIdPrefix}-${item.id}`}
            checked={selected.includes(item.id)}
            onChange={(event) =>
              onChange(event.target.checked ? [...selected, item.id] : selected.filter((id) => id !== item.id))
            }
            className="mt-1"
          />
          <span>
            {item.label}
            <span className="mt-0.5 block text-xs text-muted-foreground">{item.source[kind]}</span>
          </span>
        </label>
      ))}
      <p className="text-xs text-muted-foreground">
        复制一次，之后两边各自独立。只补这个环境里还没有的，已有的不动；凭据（env、apiKeyHelper）不带，含明文凭据的
        MCP 条目也不带（需要的话在 MCP 页逐条复制）。技能在技能页按环境分配。
      </p>
    </div>
  );
}
