# 故障排查

本文面向当前 Rust/Tauri 实现。生效配置以各助手真实文件为准；Agent Manager 自身的偏好、归属记录与未生效备选在 `native-manager.db`，设置在 `app_settings` 表。旧 Python、`manage.sh`、三层 JSON 和 profile 同步入口已退役，不是排查步骤。

## 先确认开发环境

在仓库根目录运行：

```sh
cargo --version
node --version
pnpm --version
cargo check --manifest-path engine-rs/Cargo.toml --all-targets
cargo check --manifest-path app/src-tauri/Cargo.toml --all-targets
```

前端问题在 `app/` 运行 `pnpm typecheck`、`pnpm test:unit run` 或 `pnpm build`。

## 数据库不存在或读取为空

使用隔离目录检查，不覆盖 HOME：

```sh
export AGENT_MANAGER_DATA_DIR="$(mktemp -d)"
cargo run --manifest-path engine-rs/Cargo.toml -- db:status --json
cargo run --manifest-path engine-rs/Cargo.toml -- policy:get --json
```

缺库时 `db:status` 应报告不存在，`policy:get` 应返回 Rust 内置默认值且不创建数据库。保存才会初始化 `native-manager.db`。

## 设置没有保存

设置必须通过 Tauri 设置页保存，并由 Rust bridge 写入 `app_settings` 表。检查数据目录、数据库写权限，并在保存后重新打开设置页或重启应用读回。查询与 status 只读零写。

## Policy 来源不符合预期

Policy UI 只显示 `builtin` 和 `sqlite`。编辑请求固定使用 `--layer local`，其持久化结果显示为 `sqlite`。旧共享层、同步层和 JSON settings 不再参与有效值计算。

## 执行动作失败

保存定义不会执行安装、卸载、链接或同步。执行问题按页面显示的引擎 action 排查：

1. 确认 `status` 返回的 observed 状态。
2. 执行按钮点击即真实 `apply`；`apply` 内部 preflight（写前校验）检测不适用、未接管与外部改动并 fail-closed，envelope 错误会给出拒绝类别。
3. 查看页面错误详情和 operation log；不要手动拼接旧命令。

确认弹窗只用于定义移除、清空日志等破坏性动作；普通同步、启停、更新没有二次确认步骤。

Agent、Skills、MCP、Rules、Hooks、Aliases 与实例原生域已接线，生效配置都以各助手的实际文件为准：status 只读零写，输出实际内容与写入基准 `base`；写入带 `--base`，文件在打开后被外部改过时拒绝并提示「已在外部修改」，刷新后重新操作即可，不再有「接管 = 用库覆盖」。实际安装来源不可管理、Agent 已禁用、offline 阻断或动作不适用时，引擎不会提供或执行对应 action。Skills 集合与技能分配通过内容快照与安全软连接落地，遇外部非受管内容或冲突时 fail-closed 拒绝覆盖。MCP 支持本地 stdio 与远程 HTTP，`mcp:status` 直接列出各助手配置文件里的条目，`mcp:apply` 逐条编辑、启停、复制、删除；Claude 停用的条目以快照存库（疑似凭据转存到外部密钥文件、库里只留 `${VAR}`），Codex/OpenCode 用官方 `enabled = false`。Runs 操作记录基于 SQLite `operation_logs` 提供历史审计与二次确认清空，保留期由 `log_retention_days` 控制。

公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝；执行前请用 `<domain>:status` 确认状态。

验收基线提交 `5a110be3` 的候选包只证明旧版 plan→confirm 交互，不证明本轮直接 apply 新交互。本轮源码、浏览器、隔离 CLI 与未提交候选 dirty DMG 验收已完成；最终 clean DMG 已在最终提交 `a01d0f5` 后重建并通过主审独立 verify（SHA256 与验证结果见 [交付回执](goal-rust-gui.md)）。2026-09-09 Darwin x86_64 DMG 已通过只读挂载、仓库外复制、无 Python、路径脱敏和从 `/` 隔离环境 5 秒启动验收；该包早于当前 Rules/MCP/Hooks/Secrets 与只读 `system:audit` 实现，不能证明当前 Rules/MCP/Hooks/Secrets/system:audit GUI 业务闭环。签名、公证、arm64、Linux、Windows 和更新发布仍未验证。

`system:audit` 已接入 Rust dispatcher：只读聚合 agents/skills/mcp/rules/hooks/aliases/secrets 与 runtime/storage 状态。密钥取值优先读取「密钥」页选择的外部 `.env` 文件，再回退进程环境；数据库只保存路径。若页面显示“文件不可用”，检查它是否存在、是否是普通文件、是否为 UTF-8 `KEY=VALUE`，以及 Unix 权限是否为 `600`。应用不会自动创建文件或修改权限，也不会回显取值。历史数据库明文需在页面输入确认词后显式迁移，失败时旧表保留。

## Claude Code 的额度停在很多天前

助手卡片上的额度读自本机文件，不联网。Claude 这一侧有两个来源，取较新的那个：

| 来源 | 什么时候更新 |
| --- | --- |
| `<数据目录>/usage/<编码后的配置目录>.json` | 状态栏每次渲染（事件驱动，300ms 防抖） |
| `~/.claude.json` 的 `cachedUsageUtilization` | 只在 Claude Code 主动查额度时；**常规对话不触发** |

只有后者时，数字会停在上次查额度的时刻——连用几天都不动是正常现象，不是读错了。
卡片上会照实写「N 天前更新」，过了重置点的窗口不再显示百分比。

要拿到与 Claude Code 状态栏同屏的实时值，有两条路：

**用本工具维护的状态栏**：助手编辑弹窗里把「状态栏」选成「用本工具的」，保存即部署。
它显示模型、目录、分支、上下文、花费与 5h/7d 额度，并顺手把额度存下来。已经有自己的
状态栏就别选这个——选了会把 `statusLine` 指到本工具那份（且因为那一项已有内容，
第一次写入会要求你显式接管）。

**保留自己的状态栏**：在自己的脚本里把额度顺手存一份。
Claude Code 每次渲染状态栏都会把 `rate_limits` 通过 stdin 推给脚本，但自己不写任何
文件，外部进程读不到；脚本存下来，本工具就能读到。在解析出用量之后加这一段：

```bash
# $five / $week 是已用百分比，$five_reset / $week_reset 是 Unix 秒的重置时刻，
# $now 是当前时刻（这几个变量名按你自己脚本里的实际命名改）
# 存到 Agent Manager 数据目录（macOS 默认 ~/Library/Application Support/agent-manager，
# Linux 为 ${XDG_DATA_HOME:-~/.local/share}/agent-manager；设置了 AGENT_MANAGER_DATA_DIR 时以它为准），
# 文件名是配置目录编码：先 % -> %25，再 / -> %2F。不要写进助手自己的配置目录。
if [[ -n $five || -n $week ]]; then
  am_dir=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
  am_pct='%'
  am_key=${am_dir//$am_pct/%25}
  am_key=${am_key//\//%2F}
  am_usage="$HOME/Library/Application Support/agent-manager/usage"
  [[ -d $am_usage ]] || mkdir -p "$am_usage" 2>/dev/null
  am_file=$am_usage/$am_key.json
  am_tmp=$am_file.$$
  {
    printf '{"at":%s' "${now:-0}"
    [[ -n $five ]] && printf ',"five_hour":{"used_percentage":%s,"resets_at":%s}' "$five" "${five_reset:-null}"
    [[ -n $week ]] && printf ',"seven_day":{"used_percentage":%s,"resets_at":%s}' "$week" "${week_reset:-null}"
    printf '}\n'
  } > "$am_tmp" 2>/dev/null && mv -f "$am_tmp" "$am_file" 2>/dev/null || rm -f "$am_tmp" 2>/dev/null
fi
```

写临时文件再 `mv` 覆盖，多个窗口并发也不会让本工具读到半截内容；文件写坏了本工具
自动回退到缓存那份。路径跟随 `CLAUDE_CONFIG_DIR`，所以每个实例各存各的，不会串。
`rate_limits` 只对 Pro/Max 订阅出现，且要等会话里第一次 API 响应之后才有。

Codex 那一侧不用配：它的额度由服务端随每次响应写进会话记录，用它就会更新。

## 安装渠道

Claude Code 与 Codex 的内置默认渠道是官方 `native` 命令。brew 只在实际检测结果中作为来源展示，不是默认渠道；若 observed 与预期不符，应检查本机 PATH 和实际安装来源。

## 验证

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
cd app && pnpm test:unit run && pnpm typecheck && pnpm build
./scripts/ci-smoke.sh
```
