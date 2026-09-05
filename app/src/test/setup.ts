import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import { setEngineClient } from "../lib/engine";

afterEach(() => {
  cleanup();
  setEngineClient(null);
});
