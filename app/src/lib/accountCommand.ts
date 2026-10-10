import { runEngine } from "./engine";
import type { AuthShell } from "./profileDefinition";

export interface AccountCommand {
  controlled: boolean;
  launch_value: string;
  terminal_command: string;
}

/** The engine binds account ID, trusted HOME, data directory and executable.
 * Never reconstruct a CLI invocation from a directory or the user's aliases. */
export async function accountCommand(
  element: string,
  operation: "launch" | "login" | "logout" | "status",
  shell: AuthShell = "posix",
  definition?: Record<string, unknown>,
): Promise<AccountCommand> {
  const envelope = await runEngine<AccountCommand>("accounts:command", [
    "--only", element, "--operation", operation, "--shell", shell,
    ...(definition ? ["--definition", JSON.stringify(definition)] : []),
  ]);
  if (!envelope.ok) throw new Error(envelope.errors[0]?.message ?? "账号检查失败，未生成命令");
  if (envelope.data.controlled !== true || !envelope.data.launch_value || !envelope.data.terminal_command) {
    throw new Error("没有取得受控账号命令，请刷新后重试");
  }
  return envelope.data;
}

/** A new API account never adopts a preset's or another account's key reference. */
export function accountSecretVariable(id: string, nonce: string): string {
  return `AGENT_MANAGER_ACCOUNT_${id.replace(/[^a-zA-Z0-9]/g, "_").toUpperCase()}_${nonce.replace(/[^a-zA-Z0-9]/g, "").toUpperCase()}_API_KEY`;
}
