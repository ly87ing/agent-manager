import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const root = new URL("../", import.meta.url);
const packageJson = JSON.parse(readFileSync(new URL("app/package.json", root), "utf8"));
const pnpmLock = readFileSync(new URL("app/pnpm-lock.yaml", root), "utf8").replaceAll("\r\n", "\n");
const cargoLock = readFileSync(new URL("app/src-tauri/Cargo.lock", root), "utf8").replaceAll("\r\n", "\n");
const cargoManifest = readFileSync(new URL("app/src-tauri/Cargo.toml", root), "utf8");

// Tauri's build command rejects JS/Rust plugin major/minor mismatches, even
// when each side compiles separately. Check the resolved versions before bundling.
for (const name of ["notification", "updater"]) {
  test(`Tauri ${name} plugin lockfiles have compatible versions`, () => {
    const rust = cargoLock.match(new RegExp(`name = "tauri-plugin-${name}"\\nversion = "([^"\\n]+)"`))?.[1];
    const js = pnpmLock.match(new RegExp(`      '@tauri-apps/plugin-${name}':\\n        specifier: [^\\n]+\\n        version: ([0-9.]+)`))?.[1];
    assert.ok(rust, `missing Rust ${name} plugin lock entry`);
    assert.ok(js, `missing JavaScript ${name} plugin lock entry`);
    assert.equal(js.split(".").slice(0, 2).join("."), rust.split(".").slice(0, 2).join("."),
      `JavaScript ${name} ${js} and Rust ${name} ${rust} cannot be bundled together`);
    if (name === "notification") {
      assert.equal(packageJson.dependencies["@tauri-apps/plugin-notification"], js,
        "pin notification to the tested Rust-compatible version");
      assert.ok(cargoManifest.includes(`tauri-plugin-notification = "=${rust}"`),
        "pin the Rust notification dependency to the tested JavaScript-compatible version");
    }
  });
}
