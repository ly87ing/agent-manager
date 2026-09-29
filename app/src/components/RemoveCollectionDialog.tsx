import { useState } from "react";
import { ConfirmModal } from "./ConfirmModal";

/**
 * 移除一个技能集合（来源）：列清会发生什么，git 集合默认一并清理下载的仓库与没人再用的技能文件；
 * 要输入集合名确认。
 * 技能页的集合行与规则页的规则库共用这一个确认框。
 */
export function RemoveCollectionDialog({
  name,
  local,
  executing,
  onConfirm,
  onClose,
}: {
  name: string;
  /** 本机目录来源：目录本身一律不动，没有可清理的下载。 */
  local: boolean;
  executing: boolean;
  onConfirm: (purge: boolean) => void;
  onClose: () => void;
}) {
  const [purge, setPurge] = useState(!local);
  return (
    <ConfirmModal
      title={`移除来源 ${name}`}
      description="确认后执行，想再用时可以重新添加："
      risk="install-remove"
      requireText={name}
      confirmLabel="移除"
      executing={executing}
      onConfirm={() => onConfirm(purge && !local)}
      onClose={onClose}
    >
      <ul data-testid="remove-collection-effects" className="mt-3 list-disc space-y-1.5 pl-5 text-sm">
        <li>各账号里这个集合的技能链接全部移除；你自己放进账号的同名技能目录不动</li>
        <li>用过它的规则：规则文件内容保留，只是不再跟随它、不再提示更新</li>
        <li>从它添加的启动命令已写进终端配置，保留不动</li>
        {local && <li>它是本机目录，目录里的文件一律不动</li>}
      </ul>
      {!local && (
        <label className="mt-3 flex items-center gap-2 text-sm">
          <input type="checkbox" data-testid="remove-collection-purge" checked={purge} onChange={(event) => setPurge(event.target.checked)} />
          同时清理下载的仓库和不再用到的技能文件
        </label>
      )}
    </ConfirmModal>
  );
}
