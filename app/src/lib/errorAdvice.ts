import { t } from "./i18n";
/**
 * 把常见的出错原文翻成「出了什么事 / 该怎么办」。认不出的返回 null，界面照原文显示；
 * 原文始终保留在详情里，便于对照任务记录。
 */
export interface ErrorAdvice {
  what: string;
  todo: string;
}

const RULES: [RegExp, ErrorAdvice][] = [
  [
    /已在外部修改|被外部修改|外部改过|被别处改过|与打开时读到的内容不同|base mismatch/i,
    { get what() { return t("文件在你打开之后被别处改过，这次没有写入"); }, get todo() { return t("页面已按文件的最新内容刷新，看一眼再重做一次"); } },
  ],
  [/符号链接|symlink/i, { get what() { return t("目标是一个链接（指向别处的文件），为安全起见没有操作"); }, get todo() { return t("把它换成普通文件，或直接去它指向的位置修改"); } }],
  // git 的登录失败要排在「没有权限」前面：它的原文里也有 Permission denied
  [
    /authentication failed|repository not found|could not read username|permission denied \(publickey\)/i,
    { get what() { return t("仓库拉取被拒绝：地址不对，或是私有仓库还没登录"); }, get todo() { return t("核对仓库地址；私有仓库先在终端里配好 git 登录再重试"); } },
  ],
  [/permission denied|EACCES|EPERM|没有权限|权限不足/i, { get what() { return t("没有权限读写这个文件"); }, get todo() { return t("检查文件权限，或关掉正在占用它的程序后重试"); } }],
  [
    /could not resolve host|network is unreachable|connection (refused|reset|timed out)|operation timed out|无法连接/i,
    { get what() { return t("连不上网络或远程仓库"); }, get todo() { return t("检查网络后重试；公司网络可能需要代理"); } },
  ],
  [/解析失败|parse error|invalid json|语法错误/i, { get what() { return t("配置文件的格式有错，读不出来"); }, get todo() { return t("用编辑器打开这个文件修好格式（多半是少了逗号或括号），再回来刷新"); } }],
  [/疑似凭据|明文取值|明文凭据/, { get what() { return t("内容里像是写了密钥"); }, get todo() { return t("改写成 ${变量名}，把取值放进密钥文件（设置 → MCP 密钥）"); } }],
  [/no such file or directory/i, { get what() { return t("要用的文件或目录不存在"); }, get todo() { return t("核对资源路径与来源是否存在；助手扩展路径可在助手页查看"); } }],
];

export function adviseError(message: string): ErrorAdvice | null {
  for (const [pattern, advice] of RULES) if (pattern.test(message)) return advice;
  return null;
}

/** toast 用的一句话：认得出就说人话，并带上怎么办；否则原文。 */
export function errorSummary(message: string): { text: string; description?: string } {
  const advice = adviseError(message);
  return advice ? { text: advice.what, description: t("怎么办：{0}", advice.todo) } : { text: message };
}
