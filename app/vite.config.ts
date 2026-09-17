/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 开发约定：固定 1420 端口，前端产物输出到 dist/（tauri.conf.json 的 frontendDist 指向它）。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "dist",
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    css: false,
    restoreMocks: true,
    // jsdom 环境创建开销大，默认并行度下宿主机资源竞争曾致 5s 默认超时批量误判失败；
    // 放宽超时容忍度（真实失败仍会失败，只是允许慢环境跑完）。
    testTimeout: 20000,
    hookTimeout: 20000,
  },
});
