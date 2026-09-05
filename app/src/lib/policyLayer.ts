/** 策略编辑器「默认落点层」偏好：skills 矩阵点击切换等快捷写路径落到这里。 */
import { useSyncExternalStore } from "react";
import type { PolicyLayer } from "./contract";

const STORAGE_KEY = "agent-manager.default-policy-layer";
const listeners = new Set<() => void>();

/** 默认仅本机层：个人开关不污染共享层，共享落点须在策略编辑器显式选择。 */
export function getDefaultPolicyLayer(): PolicyLayer {
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "synced" ? "synced" : "local";
  } catch {
    return "local";
  }
}

export function setDefaultPolicyLayer(layer: PolicyLayer): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, layer);
  } catch {
    // localStorage 不可用（极端环境）时保持内存态
  }
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useDefaultPolicyLayer(): [PolicyLayer, (layer: PolicyLayer) => void] {
  const layer = useSyncExternalStore(subscribe, getDefaultPolicyLayer);
  return [layer, setDefaultPolicyLayer];
}
