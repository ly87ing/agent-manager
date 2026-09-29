import { useState } from "react";
import { ConfirmModal } from "./ConfirmModal";

/**
 * 改成我的版本：把技能现在用着的那一版复制到用户目录、登记为用户的本机集合，
 * 并把勾选的账号从原版本换到用户的版本。原集合以后更新不会改用户的文件，只会提示。
 */
export function ForkSkillDialog({
  skill,
  collection,
  accounts,
  executing,
  onConfirm,
  onClose,
}: {
  skill: string;
  collection: string;
  /** 现在用着这个技能的账号（键 + 名字）。 */
  accounts: { id: string; name: string }[];
  executing: boolean;
  onConfirm: (input: { dest: string; target: string; switchAccounts: string[] }) => void;
  onClose: () => void;
}) {
  const [dest, setDest] = useState("~/my-skills");
  const [target, setTarget] = useState("my-skills");
  const [picked, setPicked] = useState<string[]>(accounts.map((account) => account.id));
  const destOk = /^(~\/|\/)/.test(dest.trim());
  const targetOk = /^[a-z0-9][a-z0-9-]*$/.test(target) && target !== collection;
  return (
    <ConfirmModal
      title={`把 ${skill} 改成我的版本`}
      description={`复制一份到你的目录，之后在那里改；${collection} 以后更新不会改你的版本，出新版时会提示你对比。`}
      risk="write-local"
      confirmLabel="复制并换过去"
      confirmDisabled={!destOk || !targetOk}
      executing={executing}
      onConfirm={() => onConfirm({ dest: dest.trim(), target, switchAccounts: picked })}
      onClose={onClose}
    >
      <div className="mt-3 space-y-3 text-sm">
        <label className="block">
          <span className="text-xs text-muted-foreground">放到哪个目录（会在里面建 {skill}/）</span>
          <input
            data-testid="fork-dest"
            value={dest}
            onChange={(event) => setDest(event.target.value)}
            className="mt-1 w-full rounded-md border border-input bg-background px-2 py-1.5 font-mono text-xs"
          />
        </label>
        <label className="block">
          <span className="text-xs text-muted-foreground">登记成哪个集合（已有同名集合须指向同一个目录）</span>
          <input
            data-testid="fork-collection"
            value={target}
            onChange={(event) => setTarget(event.target.value)}
            className="mt-1 w-full rounded-md border border-input bg-background px-2 py-1.5 font-mono text-xs"
          />
        </label>
        {accounts.length > 0 && (
          <div>
            <span className="text-xs text-muted-foreground">这些账号现在用着它，换成你的版本：</span>
            <div className="mt-1 flex flex-wrap gap-3">
              {accounts.map((account) => (
                <label key={account.id} className="flex items-center gap-1.5 text-sm">
                  <input
                    type="checkbox"
                    data-testid={`fork-switch-${account.id}`}
                    checked={picked.includes(account.id)}
                    onChange={() => setPicked((prev) => (prev.includes(account.id) ? prev.filter((id) => id !== account.id) : [...prev, account.id]))}
                  />
                  {account.name}
                </label>
              ))}
            </div>
          </div>
        )}
        {!destOk && <p className="text-xs text-destructive">目录要写绝对路径或以 ~/ 开头</p>}
        {!targetOk && <p className="text-xs text-destructive">集合名只能用小写字母、数字和连字符，且不能是 {collection}</p>}
      </div>
    </ConfirmModal>
  );
}
