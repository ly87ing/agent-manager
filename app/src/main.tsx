import { QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { AppErrorBoundary } from "./components/AppErrorBoundary";
import { initEngineClient } from "./lib/engine";
import { ExecutionProvider } from "./lib/execution";
import { createQueryClient } from "./lib/queries";
import { initTheme, ThemedToaster, ThemeProvider } from "./lib/theme";
import "./index.css";

// 首屏防闪烁：render 前同步预置 .dark（生产 CSP 不允许 index.html 内联脚本）
initTheme();

const queryClient = createQueryClient();

async function bootstrap() {
  // dev-only：?mock=1 / ?mock=offline 切换为内置 fixture 引擎（生产构建无此代码路径）
  await initEngineClient();
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <AppErrorBoundary>
        <QueryClientProvider client={queryClient}>
          <ExecutionProvider>
            <ThemeProvider>
              <App />
              <ThemedToaster />
            </ThemeProvider>
          </ExecutionProvider>
        </QueryClientProvider>
      </AppErrorBoundary>
    </StrictMode>,
  );
}

void bootstrap();
