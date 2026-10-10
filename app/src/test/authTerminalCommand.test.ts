import { describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { authTerminalCommand, defaultAuthShell, type ProfileKind } from "../lib/profileDefinition";

const kinds: ProfileKind[] = ["claude", "codex"];
const envOf = (kind: ProfileKind) => kind === "claude" ? "CLAUDE_CONFIG_DIR" : "CODEX_HOME";
const quotePs = (value: string) => `'${value.replace(/'/g, "''")}'`;

describe("authentication shell selection", () => {
  it("defaults Windows to PowerShell and Unix to POSIX", () => {
    expect(defaultAuthShell("Win32")).toBe("powershell");
    expect(defaultAuthShell("MacIntel")).toBe("posix");
    expect(defaultAuthShell("Linux x86_64")).toBe("posix");
  });

  it("quotes PowerShell executable and paths literally and uses the call operator", () => {
    expect(authTerminalCommand("claude", "C:\\My Apps\\claude.exe", "login", undefined, "powershell"))
      .toBe("& 'C:\\My Apps\\claude.exe' auth login");
    const command = authTerminalCommand("codex", "C:\\A'B\\codex.cmd", "logout", "C:\\Work's $x; & data", "powershell");
    expect(command).toContain("$env:CODEX_HOME = 'C:\\Work''s $x; & data'");
    expect(command).toContain("& 'C:\\A''B\\codex.cmd' logout");
    expect(command).toContain("finally { $env:CODEX_HOME = $previous }");
    expect(authTerminalCommand("codex", "codex", "login", "~\\My files", "powershell"))
      .toContain("(Join-Path $HOME 'My files')");
  });
});

// These tests run the generated command in the native shell in the existing
// three-platform frontend CI matrix, with disposable fake assistants only.
describe.skipIf(process.platform === "win32")("POSIX authentication command execution", () => {
  it.each(kinds)("passes %s its own directory and auth arguments without changing the parent", (kind) => {
    const root = mkdtempSync(join(tmpdir(), "am-auth-"));
    try {
      const bin = join(root, "assistant ' $ &");
      writeFileSync(bin, `#!/bin/sh\nprintf '%s\\n' "$${envOf(kind)}" "$@"\n`, { mode: 0o755 });
      for (const configDir of ["~/.work ' $(echo injected) `echo x` $data", join(root, "account ' $ & ;")]) {
        const command = authTerminalCommand(kind, bin, "login", configDir, "posix");
        const result = spawnSync("/bin/sh", ["-c", `${command}; printf 'AFTER=%s\\n' "$${envOf(kind)}"`], {
          encoding: "utf8", env: { ...process.env, HOME: root, [envOf(kind)]: "original" }, timeout: 5000,
        });
        expect(result.error).toBeUndefined();
        expect(result.status, result.stderr).toBe(0);
        expect(result.stdout.trimEnd().split("\n")).toEqual([
          configDir.startsWith("~/") ? root + configDir.slice(1) : configDir,
          ...(kind === "claude" ? ["auth"] : []), "login", "AFTER=original",
        ]);
      }
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});

// AM_TEST_POWERSHELL optionally exercises the same shell semantics on Unix;
// it does not stand in for the native Windows CI leg.
const powershell = process.platform === "win32" ? "powershell.exe" : process.env.AM_TEST_POWERSHELL;
describe.skipIf(!powershell)("PowerShell authentication command execution", () => {
  it.each(kinds)("passes %s literal paths/arguments and restores environment on success and failure", (kind) => {
    const root = mkdtempSync(join(tmpdir(), "am-auth-"));
    try {
      const shellEnv = { ...process.env, HOME: root, XDG_CONFIG_HOME: join(root, "config"), XDG_CACHE_HOME: join(root, "cache") };
      const bin = join(root, "assistant ' $ &.ps1");
      writeFileSync(bin, `[Console]::WriteLine(($args | ConvertTo-Json -Compress)); [Console]::WriteLine($env:${envOf(kind)})`);
      for (const original of ["original", null]) {
        for (const dir of [join(root, "account ' $x; & %PATH% !data"), "~/.account ' $x"]) {
          const command = authTerminalCommand(kind, bin, "logout", dir, "powershell");
          const script = `$env:${envOf(kind)} = ${original === null ? "$null" : quotePs(original)}; ${command}; [Console]::WriteLine('AFTER=' + $env:${envOf(kind)}); [Console]::WriteLine($HOME)`;
          const result = spawnSync(powershell!, ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script], { encoding: "utf8", timeout: 10000, env: shellEnv });
          expect(result.error).toBeUndefined();
          expect(result.status, result.stderr).toBe(0);
          const lines = result.stdout.trimEnd().split(/\r?\n/);
          const args = JSON.parse(lines[0]);
          expect(Array.isArray(args) ? args : [args]).toEqual([...(kind === "claude" ? ["auth"] : []), "logout"]);
          expect(lines[1]).toBe(dir.startsWith("~/") ? join(lines[3], dir.slice(2)) : dir);
          expect(lines[2]).toBe(`AFTER=${original ?? ""}`);
        }
      }
      writeFileSync(bin, "throw 'expected failure'");
      const command = authTerminalCommand(kind, bin, "login", "~/.work ' $x", "powershell");
      const result = spawnSync(powershell!, ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command",
        `$env:${envOf(kind)} = 'original'; try { ${command} } catch { }; [Console]::WriteLine($env:${envOf(kind)})`], { encoding: "utf8", timeout: 10000, env: shellEnv });
      expect(result.status, result.stderr).toBe(0);
      expect(result.stdout.trim()).toBe("original");
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});


describe.skipIf(process.platform !== "win32")("Windows npm-style executable authentication", () => {
  it.each(kinds)("executes a quoted .cmd path for %s with the correct environment and arguments", (kind) => {
    const root = mkdtempSync(join(tmpdir(), "am-auth-cmd-"));
    try {
      const bin = join(root, "assistant with spaces.cmd");
      const probe = join(root, "probe.cjs");
      const dir = join(root, "account ' $x; & %PATH% !data");
      writeFileSync(probe, `console.log(JSON.stringify({ args: process.argv.slice(2), dir: process.env.${envOf(kind)} }));`);
      writeFileSync(bin, '@echo off\r\n"%AM_AUTH_NODE%" "%AM_AUTH_PROBE%" %*\r\n');
      const result = spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", authTerminalCommand(kind, bin, "login", dir, "powershell")], {
        encoding: "utf8", timeout: 10000, env: { ...process.env, AM_AUTH_NODE: process.execPath, AM_AUTH_PROBE: probe },
      });
      expect(result.error).toBeUndefined();
      expect(result.status, result.stderr).toBe(0);
      expect(JSON.parse(result.stdout.trim())).toEqual({ args: [...(kind === "claude" ? ["auth"] : []), "login"], dir });
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});
