import { useSyncExternalStore } from "react";
import { messages } from "./messages";

export type Language = "en" | "zh-CN";
let language: Language = "en";
const listeners = new Set<() => void>();

export function getLanguage(): Language { return language; }

export function setLanguage(next: Language): void {
  if (language === next) return;
  language = next;
  document.documentElement.lang = next;
  for (const listener of listeners) listener();
}

export function useLanguage(): Language {
  return useSyncExternalStore(
    (listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    () => language,
  );
}

/** Only application-owned copy belongs here. Never translate user or engine data. */
export function t(key: keyof typeof messages, ...values: unknown[]): string {
  const text = language === "en" ? messages[key] : key;
  return text.replace(/\{(\d+)\}/g, (_, index: string) => String(values[Number(index)]));
}
