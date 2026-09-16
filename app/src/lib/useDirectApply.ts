import { useRef, useState } from "react";
import type { Envelope } from "./contract";
import { useExecutions } from "./execution";

/**
 * 写操作直执入口：点击即真实 apply，不开预览弹窗。
 * run_id 记录、引擎查询刷新、成功/失败 toast 由 useExecutions 统一处理；
 * 同步 ref 保证同一 hook 实例在途期间忽略重复提交（防连点）。
 */
export function useDirectApply() {
  const { execute } = useExecutions();
  const busyRef = useRef(false);
  const [executing, setExecuting] = useState(false);

  const apply = (request: { command: string; label: string; args?: string[] }): Promise<Envelope | null> => {
    if (busyRef.current) return Promise.resolve(null);
    busyRef.current = true;
    setExecuting(true);
    return execute({
      command: request.command,
      args: request.args ?? [],
      label: request.label,
    }).finally(() => {
      busyRef.current = false;
      setExecuting(false);
    });
  };

  return { apply, executing };
}
