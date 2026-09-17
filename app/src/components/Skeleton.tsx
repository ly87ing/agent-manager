import { cn } from "../lib/utils";

/** 加载占位块：pulse 动画 + muted token 色，组合成与真实卡片轮廓对齐的骨架屏。 */
export function Skeleton({ className }: { className?: string }) {
  return <div aria-hidden className={cn("animate-pulse rounded-md bg-muted", className)} />;
}
