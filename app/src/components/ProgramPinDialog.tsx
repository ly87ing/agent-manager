import { useEffect, useState } from "react";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import { ConfirmModal } from "./ConfirmModal";
import { FileDiff } from "./FileDiff";
import type { EngineElement, StatusData } from "../lib/contract";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { useExecutions } from "../lib/execution";
import type { McpAgentCell, McpObserved } from "../lib/observed";
import { isExactVersion, pinArgs, shortVersion, type ProgramElement } from "../lib/programs";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { useAccountNames } from "../lib/useAccountNames";

interface Target {
  server: string;
  agent: string;
  cell: McpAgentCell;
  runner: "npx" | "uvx";
  args: string[] | null;
}

/**
 * 按需拉取（npx / uvx）的 MCP 服务：改条目里写的版本——固定到某个版本或升级到新版本。
 * 只改启动参数里的版本那一段，其余字段原样（走 MCP 的编辑：写前比对、只动这个条目）。
 * 每个账号的改动先预览（只显示改动的行），确认后逐个写入，一个失败不拦其他。
 */
export function ProgramPinDialog({ program, initialVersion, onClose }: { program: ProgramElement; initialVersion: string | null; onClose: () => void }) {
  const mcp = useEngineQuery<StatusData>("mcp:status");
  const nameOf = useAccountNames();
  const { execute } = useExecutions();
  const queryClient = useQueryClient();
  const [version, setVersion] = useState(initialVersion ?? "");
  const [previews, setPreviews] = useState<Record<string, { path: string; original: string | null; content: string } | string>>({});
  const [executing, setExecuting] = useState(false);

  const elements: EngineElement[] = mcp.data?.ok ? mcp.data.data.elements ?? [] : [];
  const targets: Target[] = program.runners.flatMap((runner) => {
    const element = elements.find((item) => item.id === `mcp:${runner.server}`);
    const cells = (element?.observed as McpObserved | undefined)?.agents ?? {};
    const dependent = program.dependents.find((dep) => dep.kind === "mcp" && dep.name === runner.server);
    return (dependent?.agents ?? [runner.agent]).flatMap((agent) => {
      const cell = cells[agent];
      if (!cell?.entry || !cell.actions.includes("edit")) return [];
      const args = version.trim() ? pinArgs(runner.runner, cell.entry.args ?? [], version.trim()) : null;
      return [{ server: runner.server, agent, cell, runner: runner.runner, args }];
    });
  });
  const valid = version.trim() !== "" && isExactVersion(version.trim());
  const specOf = (target: Target) => {
    const entry = target.cell.entry!;
    const spec: Record<string, unknown> = { command: entry.command, args: target.args, env: entry.env ?? {} };
    if (entry.startup_timeout_sec !== undefined) spec.startup_timeout_sec = entry.startup_timeout_sec;
    return spec;
  };
  const argsOf = (target: Target) => [
    "--only", `mcp:${target.server}`, "--agent", target.agent, "--action", "edit", "--base", target.cell.base, "--spec", JSON.stringify(specOf(target)),
  ];
  const key = targets.map((target) => `${target.server}@${target.agent}:${target.args?.join(" ")}`).join("|");

  useEffect(() => {
    if (!valid) {
      setPreviews({});
      return;
    }
    let alive = true;
    void Promise.all(
      targets.map(async (target) => {
        const id = `${target.server}@${target.agent}`;
        if (!target.args) return [id, "认不出条目里写版本的位置，没法自动改；可以在 MCP 页手动编辑"] as const;
        try {
          const envelope = await runEngine<{ files: { path: string; original?: string | null; content: string }[] }>("mcp:preview", argsOf(target));
          const file = envelope.ok ? envelope.data.files[0] : null;
          return [id, file ? { path: file.path, original: file.original ?? null, content: file.content } : envelope.errors[0]?.message ?? "预览失败"] as const;
        } catch (reason) {
          return [id, engineErrorMessage(reason)] as const;
        }
      }),
    ).then((entries) => alive && setPreviews(Object.fromEntries(entries)));
    return () => {
      alive = false;
    };
  }, [key, valid]);

  const confirm = async () => {
    setExecuting(true);
    let failed = 0;
    try {
      for (const target of targets.filter((item) => item.args)) {
        const envelope = await execute({ command: "mcp:apply", label: `把 ${target.server} @ ${nameOf(target.agent)} 固定到 ${shortVersion(version.trim())}`, args: argsOf(target), silent: true });
        if (!envelope?.ok) failed += 1;
      }
      (failed ? toast.error : toast.success)(failed ? `${failed} 处没改成，详见操作记录` : `已固定到 ${shortVersion(version.trim())}`);
      if (!failed) onClose();
    } finally {
      setExecuting(false);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    }
  };

  const ready = valid && targets.length > 0 && targets.every((target) => typeof previews[`${target.server}@${target.agent}`] === "object");
  return (
    <ConfirmModal
      title={`${program.name}：固定 MCP 用的版本`}
      description="只改启动参数里的版本号，条目的其他内容不动。固定之后即使发布了新版本也不会被自动换掉，要升级时在这里改。"
      risk="write-local"
      confirmLabel="写入"
      confirmDisabled={!ready}
      executing={executing}
      onConfirm={() => void confirm()}
      onClose={() => !executing && onClose()}
    >
      <div data-testid="program-pin-dialog" className="mt-3 space-y-3 text-sm">
        <label className="block space-y-1">
          <span className="text-xs text-muted-foreground">版本（精确版本号，或 git 源的 40 位提交号）</span>
          <input
            data-testid="program-pin-version"
            value={version}
            onChange={(event) => setVersion(event.target.value)}
            className="w-full rounded-md border bg-card px-2 py-1.5 font-mono text-xs"
          />
          {version.trim() && !valid && <span className="text-xs text-warning">要写精确版本（如 1.10.1）或完整提交号，不能是 latest 或分支名</span>}
        </label>
        {targets.length === 0 && <p className="text-xs text-muted-foreground">没有可以改的条目（可能已停用或只保存在本工具里）。</p>}
        {targets.map((target) => {
          const preview = previews[`${target.server}@${target.agent}`];
          return (
            <div key={`${target.server}@${target.agent}`} className="space-y-1">
              <div className="text-xs font-medium">{target.server} · {nameOf(target.agent)}</div>
              {typeof preview === "string" ? (
                <p className="text-xs text-warning">{preview}</p>
              ) : preview ? (
                <FileDiff path={preview.path} original={preview.original} content={preview.content} maxHeight="max-h-40" />
              ) : valid ? (
                <p className="text-xs text-muted-foreground">正在预览…</p>
              ) : null}
            </div>
          );
        })}
      </div>
    </ConfirmModal>
  );
}
