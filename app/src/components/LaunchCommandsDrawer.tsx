import { Copy, Plus, X } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { launchCommandsOfAccount, type Account, type AgentAccounts } from "../lib/accounts";
import { useDialogFocus } from "../lib/dialogFocus";
import { fixLaunchCommandTilde } from "../lib/profileDefinition";
import { DEFAULT_LANG, DEFAULT_TIMEZONE, type useAliasSource } from "../lib/useAliasSource";
import { AliasApplyConfirm } from "./AliasApplyConfirm";
import { AliasEditorDialog } from "./AliasEditorDialog";

type AliasSource = ReturnType<typeof useAliasSource>;

/**
 * 一个账号的启动命令：逐条新增、编辑、删除，旧写法一键订正。只改 `~/.zshrc` / `~/.zprofile` 里
 * Agent Manager 区块中的那几行；区块外你自己写的 alias 只读列出。每次写入都先看完整文件预览，
 * 文件在打开后被改过就拒绝。
 */
export function LaunchCommandsDrawer({
  account,
  group,
  aliasSource,
  defaultValue,
  onClose,
}: {
  account: Account;
  group: AgentAccounts;
  aliasSource: AliasSource;
  /** 还没有命令时新增一条的底稿（按账号登记生成）。 */
  defaultValue: string;
  onClose: () => void;
}) {
  const source = aliasSource.source;
  const [editing, setEditing] = useState<{ name: string; value: string | null; presetName?: string } | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({ onEscape: () => !editing && !removing && onClose() });
  if (!source) return null;

  const external = source.targets.flatMap((target) => target.external);
  const commands = launchCommandsOfAccount(account, group, source.aliases, external);
  const previewBase = JSON.stringify(source.targets.map((target) => [target.id, target.base]));
  const files = source.targets.map((target) => target.target).join("、");

  const addNew = () => {
    const base = account.isMain ? group.bin.split("/").pop() ?? "claude" : account.elementId.replace(/^profiles:/, "");
    let candidate = commands.length === 0 ? base : `${base}-2`;
    for (let n = 3; source.aliases[candidate] !== undefined; n += 1) candidate = `${base}-${n}`;
    // 以这个账号已有的第一条为底稿：配置目录等前置变量照搬，只改名字和参数
    setEditing({ name: "", value: commands[0]?.value ?? defaultValue, presetName: candidate });
  };

  /** 只订正本工具早先生成错的目录写法（"~/…" 不会展开）；不替用户往命令里加别的东西。 */
  const fix = async (name: string, value: string) => {
    await aliasSource.setAlias(name, fixLaunchCommandTilde(value), { label: `订正启动命令 ${name}` });
  };

  return (
    <div className="fixed inset-0 z-40 flex justify-end bg-black/30" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={`${account.name} 的启动命令`}
        data-testid="launch-commands-drawer"
        onKeyDown={onDialogKeyDown}
        className="flex h-full w-full max-w-md flex-col border-l bg-card shadow-xl"
      >
        <header className="flex items-center justify-between border-b px-5 py-3">
          <h2 className="text-base font-semibold">{account.isMain ? `${group.label} 主账号` : account.name} 的启动命令</h2>
          <button type="button" aria-label="关闭" onClick={onClose} className="rounded p-1 text-muted-foreground hover:bg-accent">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </header>
        <div className="flex-1 space-y-3 overflow-y-auto px-5 py-4 text-sm">
          <p className="text-xs text-muted-foreground">
            在终端输入命令名启动这个账号（目录 <code className="font-mono">{account.configDir}</code>）。可以有多条，比如日常一条、带额外参数一条。
          </p>
          {commands.length === 0 && <p className="text-xs text-warning">还没有指向这个账号的启动命令。</p>}
          <ul className="divide-y rounded-md border">
            {commands.map((command) => (
              <li key={`${command.managed}-${command.name}`} data-testid={`launch-command-${command.name}`} className="space-y-1.5 px-3 py-2">
                <div className="flex items-center gap-2">
                  <code className="font-mono font-medium">{command.name}</code>
                  {!command.managed && <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">你自己写的，只读</span>}
                  <div className="ml-auto flex items-center gap-1">
                    <button
                      type="button"
                      aria-label={`复制 ${command.name}`}
                      onClick={() => void navigator.clipboard.writeText(command.name).then(() => toast.success(`已复制 ${command.name}`)).catch(() => toast.error("复制失败"))}
                      className="rounded p-1 text-muted-foreground hover:bg-accent"
                    >
                      <Copy className="h-3.5 w-3.5" aria-hidden />
                    </button>
                    {command.managed && (
                      <>
                        <button type="button" data-testid={`launch-edit-${command.name}`} onClick={() => setEditing({ name: command.name, value: command.value })} className="rounded px-2 py-0.5 text-xs hover:bg-accent">
                          编辑
                        </button>
                        <button type="button" data-testid={`launch-remove-${command.name}`} onClick={() => setRemoving(command.name)} className="rounded px-2 py-0.5 text-xs text-destructive hover:bg-destructive/10">
                          删除
                        </button>
                      </>
                    )}
                  </div>
                </div>
                <code className="block break-all rounded bg-muted p-1.5 font-mono text-[11px]">{`alias ${command.name}='${command.value}'`}</code>
                {command.managed && command.needsTildeFix && (
                  <div className="flex items-center gap-2 text-xs text-warning">
                    <span>目录写成了不会展开的 "~/…"，助手找不到这个账号的配置。</span>
                    <button
                      type="button"
                      data-testid={`launch-fix-${command.name}`}
                      disabled={aliasSource.busy}
                      onClick={() => void fix(command.name, command.value)}
                      className="shrink-0 rounded border border-warning/40 px-2 py-0.5 hover:bg-warning/10"
                    >
                      订正
                    </button>
                  </div>
                )}
              </li>
            ))}
          </ul>
          <p className="text-xs text-muted-foreground">只改 {files} 里由本工具管理的这几行，改完新开终端生效。</p>
        </div>
        <footer className="flex justify-end border-t px-5 py-3">
          <button
            type="button"
            data-testid="launch-add"
            disabled={!source.writable}
            onClick={addNew}
            className="inline-flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-40"
          >
            <Plus className="h-4 w-4" aria-hidden /> 新增一条
          </button>
        </footer>
      </div>

      {editing && (
        <AliasEditorDialog
          initialName={editing.value === null || editing.presetName ? undefined : editing.name}
          initialValue={editing.value ?? undefined}
          defaultCommand={group.bin}
          presetName={editing.presetName}
          defaults={{ timezone: DEFAULT_TIMEZONE, lang: DEFAULT_LANG }}
          existingNames={Object.keys(source.aliases)}
          previewBase={previewBase}
          onPreview={aliasSource.previewSetAlias}
          onRefresh={() => {
            setEditing(null);
            void aliasSource.query.refetch();
          }}
          onSubmit={(name, value) => {
            // 新增时 name 只是预填名，不算改名；编辑时改了名字就是重命名
            const renameFrom = editing.presetName ? undefined : editing.name || undefined;
            void aliasSource.setAlias(name, value, { renameFrom }).then((ok) => {
              if (ok) setEditing(null);
            });
          }}
          onClose={() => setEditing(null)}
        />
      )}
      {removing && (
        <AliasApplyConfirm
          title={`删除启动命令 ${removing}`}
          description="只删这一条命令，账号的目录与登录不受影响。确认后从终端配置里去掉。"
          confirmLabel="确认删除"
          previewBase={previewBase}
          op={["--remove", removing]}
          onPreview={aliasSource.previewApply}
          onRefresh={() => {
            setRemoving(null);
            void aliasSource.query.refetch();
          }}
          executing={aliasSource.busy}
          onConfirm={() =>
            void aliasSource.removeAlias(removing, { label: `删除启动命令 ${removing}` }).then((ok) => {
              if (ok) setRemoving(null);
            })
          }
          onClose={() => setRemoving(null)}
        />
      )}
    </div>
  );
}
