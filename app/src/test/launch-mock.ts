import type { LaunchEntry, LaunchRecipe, SavedLaunch } from "../lib/launch";
import { makeEnvelope } from "./fixtures";

/** Dev-only examples for layout checks. Native rendering/execution is tested in Rust. */
export function createLaunchMock() {
  const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
  let revision = 0;
  let saved: SavedLaunch[] = [];
  function entries(spec: LaunchRecipe): LaunchEntry[] {
    return spec.variants.map((variant) => {
      const command = spec.platform === "windows" ? `& { $env:TZ = '${variant.timezone}'; & '${spec.target}' } # mock preview`
        : (spec.kind === "app" && spec.platform === "macos" ? `open --env ${quote(`TZ=${variant.timezone}`)} -a ${quote(spec.target)}` : `TZ=${quote(variant.timezone)} command ${quote(spec.target)}`)
          + (spec.args.length ? ` ${spec.platform === "macos" && spec.kind === "app" ? "--args " : ""}${spec.args.map(quote).join(" ")}` : "");
      const definition = spec.platform === "windows" ? `function ${variant.name} { ${command} }` : `alias ${variant.name}=${quote(command)}`;
      return { ...variant, command, definition };
    });
  }
  return (command: string, args: string[]) => {
    const value = (flag: string) => args[args.indexOf(flag) + 1];
    if (command === "launch:catalog") return makeEnvelope({ platform: "macos", shell: "zsh", apps: [{ name: "Example Browser", path: "/Applications/Example Browser.app" }] }, { command });
    if (command === "launch:list") return makeEnvelope({ items: saved }, { command });
    if (command === "launch:run") return makeEnvelope({ state: "requested", timezone_verified: false }, { command });
    const built = args.includes("--spec") ? entries(JSON.parse(value("--spec")) as LaunchRecipe) : [];
    if (command === "launch:build") return makeEnvelope({ entries: built, timezone_verified: false }, { command });
    if (command === "launch:save") {
      const recipe = JSON.parse(value("--spec")) as LaunchRecipe;
      const id = args.includes("--id") ? value("--id") : `saved-${revision + 1}`;
      const item = { id, revision: (++revision).toString(16).padStart(64, "0"), recipe, entries: entries(recipe) };
      saved = [...saved.filter((old) => old.id !== id), item];
      return makeEnvelope({ item }, { command });
    }
    if (command === "launch:delete") {
      const id = value("--id"); saved = saved.filter((item) => item.id !== id);
      return makeEnvelope({ id }, { command });
    }
    throw new Error("Unknown launch mock operation");
  };
}
