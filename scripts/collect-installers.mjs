// 收集当前目标的安装包；缺少任一声明格式时失败，避免上传残缺制品。
import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";

const { BUILD_TARGET: target, BUILD_BUNDLES: bundles, RUNNER_TEMP: temp,
  AGENT_MANAGER_BUILD_COMMIT: commit, AGENT_MANAGER_BUILD_DATE: date } = process.env;
if (!target || !bundles || !temp || !/^[a-f0-9]{40}$/.test(commit ?? "") || !date) {
  throw new Error("缺少构建目标、格式、临时目录或提交信息");
}
const formats = { dmg: ".dmg", appimage: ".AppImage", deb: ".deb", nsis: ".exe", msi: ".msi" };
const root = join("app", "src-tauri", "target", target, "release", "bundle");
const files = bundles.split(",").flatMap((format) => {
  if (!formats[format]) throw new Error(`不支持的安装格式：${format}`);
  const directory = join(root, format);
  const found = readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(formats[format]))
    .map((entry) => join(directory, entry.name));
  if (!found.length) throw new Error(`缺少 ${format} 安装包`);
  return found;
});
const output = join(temp, "agent-manager-installers");
mkdirSync(output); // runner 临时目录中每次新建，拒绝混入旧文件。
const checksums = files.sort().map((file) => {
  const content = readFileSync(file);
  if (!content.length) throw new Error(`安装包为空：${basename(file)}`);
  copyFileSync(file, join(output, basename(file)));
  return `${createHash("sha256").update(content).digest("hex")}  ${basename(file)}`;
});
writeFileSync(join(output, "SHA256SUMS.txt"), `${checksums.join("\n")}\n`);
writeFileSync(join(output, "build-info.json"), `${JSON.stringify({
  commit, date, target,
  version: JSON.parse(readFileSync("app/package.json", "utf8")).version,
  releaseSigned: false,
  installers: files.map((file) => basename(file)),
}, null, 2)}\n`);
console.log(`已收集 ${files.length} 个 ${target} 安装包及校验清单`);
