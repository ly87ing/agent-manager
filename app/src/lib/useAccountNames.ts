import type { AccountGroup } from "./accounts";
import { useEngineQuery } from "./queries";

export interface ResourceTarget {
  id: string;
  name: string;
  group: string;
  kind: "claude" | "codex" | null;
  config_dir?: string | null;
  skills_dir?: string | null;
  mcp_path?: string | null;
}

/** Labels only: no account status, authentication, settings or launch commands. */
export function useResourceTargets(): ResourceTarget[] {
  const query = useEngineQuery<{ targets: ResourceTarget[] }>("resources:targets");
  return query.data?.ok ? query.data.data.targets ?? [] : [];
}

export function useAccountNames(): (id: string) => string {
  const targets = useResourceTargets();
  return (id) => targets.find((target) => target.id === id)?.name ?? id;
}

export function useAccountGroups(): (ids: string[]) => AccountGroup[] {
  const targets = useResourceTargets();
  return (ids) => {
    const groups: AccountGroup[] = [];
    for (const id of ids) {
      const target = targets.find((item) => item.id === id);
      const key = target?.group ?? id;
      let group = groups.find((item) => item.key === key);
      if (!group) {
        group = {
          key,
          label: targets.find((item) => item.id === key)?.name ?? key,
          kind: target?.kind ?? null,
          columns: [],
        };
        groups.push(group);
      }
      const name = target?.name ?? id;
      group.columns.push({ id, name, fullName: name });
    }
    return groups;
  };
}
