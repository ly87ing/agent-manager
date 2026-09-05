import { QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Toaster } from "sonner";
import App from "./App";
import { initEngineClient } from "./lib/engine";
import { createQueryClient } from "./lib/queries";
import "./index.css";

const queryClient = createQueryClient();

async function bootstrap() {
  // dev-only：?mock=1 / ?mock=offline 切换为内置 fixture 引擎（生产构建无此代码路径）
  await initEngineClient();
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <App />
        <Toaster richColors position="top-right" />
      </QueryClientProvider>
    </StrictMode>,
  );
}

void bootstrap();
