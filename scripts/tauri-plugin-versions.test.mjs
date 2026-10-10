import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const root = new URL("../", import.meta.url);
const pnpmLock = readFileSync(new URL("app/pnpm-lock.yaml", root), "utf8").replaceAll("\r\n", "\n");
const cargoLock = readFileSync(new URL("app/src-tauri/Cargo.lock", root), "utf8").replaceAll("\r\n", "\n");

// Tauri's build command rejects JS/Rust plugin major/minor mismatches, even
// when each side compiles separately. Check the resolved versions before bundling.
for (const name of ["updater"]) {
  test(`Tauri ${name} plugin lockfiles have compatible versions`, () => {
    const rust = cargoLock.match(new RegExp(`name = "tauri-plugin-${name}"\\nversion = "([^"\\n]+)"`))?.[1];
    const js = pnpmLock.match(new RegExp(`      '@tauri-apps/plugin-${name}':\\n        specifier: [^\\n]+\\n        version: ([0-9.]+)`))?.[1];
    assert.ok(rust, `missing Rust ${name} plugin lock entry`);
    assert.ok(js, `missing JavaScript ${name} plugin lock entry`);
    assert.equal(js.split(".").slice(0, 2).join("."), rust.split(".").slice(0, 2).join("."),
      `JavaScript ${name} ${js} and Rust ${name} ${rust} cannot be bundled together`);
  });
}
