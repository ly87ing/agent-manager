import { t } from "./i18n";
/**
 * git 技能来源的跟踪方式（来源定义里的 track）与版本显示。
 * 缺省为自动：仓库有版本号标签就跟最新正式版，否则跟主分支。
 */
export type TrackSpec =
  | { mode: "auto" }
  | { mode: "tag"; pattern?: string; prerelease?: boolean }
  | { mode: "default" }
  | { mode: "branch"; branch: string }
  | { mode: "branch-pattern"; pattern: string }
  | { mode: "pinned"; ref: string };

export type TrackMode = TrackSpec["mode"];

/** 引擎给出的一个版本位置：标签、分支或提交。 */
export interface VersionRef {
  commit: string;
  kind: "tag" | "branch" | "commit" | string | null;
  name: string | null;
  label: string | null;
}

export const TRACK_OPTIONS: { mode: TrackMode; label: string }[] = [
  { mode: "auto", get label() { return t("自动（有版本号跟版本，否则跟主分支）"); } },
  { mode: "tag", get label() { return t("跟最新版本（标签）"); } },
  { mode: "default", get label() { return t("跟主分支"); } },
  { mode: "branch", get label() { return t("跟指定分支"); } },
  { mode: "branch-pattern", get label() { return t("跟最新的匹配分支"); } },
  { mode: "pinned", get label() { return t("固定在某个版本"); } },
];

/** 一句话描述跟踪方式。 */
export function describeTrack(track: TrackSpec | null | undefined): string {
  const spec = track ?? { mode: "auto" };
  switch (spec.mode) {
    case "auto":
      return t("自动");
    case "tag":
      return t("最新版本{0}{1}", spec.pattern && spec.pattern !== "*" ? `（${spec.pattern}）` : "", spec.prerelease ? t("，含预发布") : "");
    case "default":
      return t("主分支");
    case "branch":
      return t("分支 {0}", spec.branch);
    case "branch-pattern":
      return t("最新的 {0}", spec.pattern);
    case "pinned":
      return t("固定在 {0}", spec.ref);
  }
}

/** 版本名字：标签与分支用名字，提交用前 7 位。 */
export function versionLabel(ref: VersionRef | null | undefined): string | null {
  if (!ref) return null;
  return ref.label ?? ref.commit.slice(0, 7);
}

/** 表单里的跟踪方式字段 → 来源定义里的 track（自动存为 null，不写进定义）。输入不完整时返回错误。 */
export function trackFromForm(form: { mode: TrackMode; value: string; prerelease: boolean }): { track: TrackSpec | null } | { error: string } {
  const value = form.value.trim();
  const valid = (text: string) => /^[A-Za-z0-9._/+*-]+$/.test(text) && !text.startsWith("-") && !text.includes("..");
  switch (form.mode) {
    case "auto":
      return { track: null };
    case "default":
      return { track: { mode: "default" } };
    case "tag":
      if (value && !valid(value)) return { error: t("版本规则只能用字母、数字与 . _ / + - *") };
      return { track: { mode: "tag", ...(value && value !== "*" ? { pattern: value } : {}), ...(form.prerelease ? { prerelease: true } : {}) } };
    case "branch":
      if (!value || !valid(value) || value.includes("*")) return { error: t("填分支名，例如 stable") };
      return { track: { mode: "branch", branch: value } };
    case "branch-pattern":
      if (!value || !valid(value) || !value.includes("*")) return { error: t("填带 * 的分支规则，例如 release/*") };
      return { track: { mode: "branch-pattern", pattern: value } };
    case "pinned":
      if (!value || !valid(value) || value.includes("*")) return { error: t("填标签、分支或提交，例如 v3.0.1") };
      return { track: { mode: "pinned", ref: value } };
  }
}

/** 来源定义里的 track → 表单初值。 */
export function trackToForm(track: TrackSpec | null | undefined): { mode: TrackMode; value: string; prerelease: boolean } {
  const spec = track ?? { mode: "auto" };
  switch (spec.mode) {
    case "tag":
      return { mode: "tag", value: spec.pattern && spec.pattern !== "*" ? spec.pattern : "", prerelease: spec.prerelease === true };
    case "branch":
      return { mode: "branch", value: spec.branch, prerelease: false };
    case "branch-pattern":
      return { mode: "branch-pattern", value: spec.pattern, prerelease: false };
    case "pinned":
      return { mode: "pinned", value: spec.ref, prerelease: false };
    default:
      return { mode: spec.mode, value: "", prerelease: false };
  }
}
