// 测试环境 hermetic：所有 toLocaleString/toLocaleTimeString 渲染固定到 UTC，
// 不依赖 runner 的 TZ（CI 与本机结果一致；真实应用仍按用户本地时区显示）。
process.env.TZ = "UTC";

import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import { setEngineClient } from "../lib/engine";

afterEach(() => {
  cleanup();
  setEngineClient(null);
});
