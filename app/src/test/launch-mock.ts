import type { LaunchEntry, LaunchRecipe, LaunchStatus } from "../lib/launch";
import { makeEnvelope } from "./fixtures";

/** Dev-only examples for layout checks. Native rendering/execution is tested in Rust. */
export function createLaunchMock() {
  const state: LaunchStatus = { platform: "macos", shell: "zsh", path: "/home/demo/.zshrc", base: "missing", entries: [], external_names: [] };
  const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
  let content = "";
  let revision = 0;
  function entries(spec: LaunchRecipe): LaunchEntry[] {
    return spec.variants.map((variant) => {
      const command = spec.platform === "windows" ? `& { $env:TZ = '${variant.timezone}'; & '${spec.target}' } # mock preview`
        : (spec.kind === "app" && spec.platform === "macos" ? `open --env ${quote(`TZ=${variant.timezone}`)} -a ${quote(spec.target)}` : `TZ=${quote(variant.timezone)} command ${quote(spec.target)}`)
          + (spec.args.length ? ` ${spec.platform === "macos" && spec.kind === "app" ? "--args " : ""}${spec.args.map(quote).join(" ")}` : "");
      return { ...variant, command, definition: `alias ${variant.name}=${quote(command)}` };
    });
  }
  return (command: string, args: string[]) => {
    const value = (flag: string) => args[args.indexOf(flag) + 1];
    if (command === "launch:catalog") return makeEnvelope({ platform: "macos", shell: "zsh", apps: [{ name: "Example Browser", path: "/Applications/Example Browser.app" }] }, { command });
    if (command === "launch:status") return makeEnvelope({ ...state, shell: value("--shell") }, { command });
    if (command === "launch:run") return makeEnvelope({ state: "requested", timezone_verified: false }, { command });
    const built = args.includes("--spec") ? entries(JSON.parse(value("--spec")) as LaunchRecipe) : [];
    if (command === "launch:build") return makeEnvelope({ entries: built, timezone_verified: false }, { command });
    const next = args.includes("--remove") ? state.entries.filter((entry) => entry.name !== value("--remove"))
      : [...state.entries.filter((entry) => !built.some((item) => item.name === entry.name)), ...built.map((entry) => ({ ...entry, path: state.path }))];
    const nextContent = next.map((entry) => entry.definition).join("\n");
    const files = [{ path: state.path, original: content || null, content: nextContent, deleted: false }];
    if (command === "launch:preview") return makeEnvelope({ base: state.base, files }, { command });
    if (command === "launch:apply") {
      if (!args.includes("--confirm")) throw new Error("Mock apply requires confirmation");
      content = nextContent; state.entries = next; state.base = (++revision).toString(16).padStart(64, "0");
      return makeEnvelope({ files, status: { ...state } }, { command });
    }
    throw new Error("Unknown launch mock operation");
  };
}
