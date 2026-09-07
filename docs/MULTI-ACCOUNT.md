# Agent CLI 多账号配置指南

在同一台机器上运行多个 Agent CLI 账号（例如个人 / 工作分离），核心思路相同：**通过环境变量将每个账号的数据目录隔离**。

## 原理总览

| 项目 | Claude Code | Kimi Code |
|------|------------|-----------|
| 数据目录环境变量 | `CLAUDE_CONFIG_DIR` | `KIMI_CODE_HOME` |
| 默认数据目录 | `~/.claude` | `~/.kimi-code` |
| 目录包含的内容 | 凭证、设置、MCP、会话、项目权限 | 凭证、config.toml、mcp.json、会话、OAuth |
| 隔离粒度 | 完全隔离（每个目录独立） | 完全隔离（每个目录独立） |

---

## Claude Code 多账号

### 1. 创建独立数据目录

```bash
mkdir -p ~/.claude-personal
mkdir -p ~/.claude-work
```

### 2. 各自登录

```bash
CLAUDE_CONFIG_DIR=$HOME/.claude-personal claude login
CLAUDE_CONFIG_DIR=$HOME/.claude-work claude login
```

### 3. 配置 Shell Alias

在 `~/.zshrc` 中添加：

```bash
# Claude 多账号
alias claude-personal='CLAUDE_CONFIG_DIR=$HOME/.claude-personal claude'
alias claude-work='CLAUDE_CONFIG_DIR=$HOME/.claude-work claude'

# 带常用参数的版本
alias cc-personal='CLAUDE_CONFIG_DIR=$HOME/.claude-personal claude --dangerously-skip-permissions'
alias cc-work='CLAUDE_CONFIG_DIR=$HOME/.claude-work claude --dangerously-skip-permissions'
```

### 4. 为不同账号使用不同 settings

Claude 支持通过 `--settings` 参数指定配置文件，可以结合使用：

```bash
alias cc-personal='CLAUDE_CONFIG_DIR=$HOME/.claude-personal claude --dangerously-skip-permissions --settings $HOME/.claude/profiles/personal.json'
alias cc-work='CLAUDE_CONFIG_DIR=$HOME/.claude-work claude --dangerously-skip-permissions --settings $HOME/.claude/profiles/work.json'
```

---

## Kimi Code 多账号

### 1. 创建独立数据目录

```bash
mkdir -p ~/.kimi-code-personal
mkdir -p ~/.kimi-code-work
```

### 2. 各自登录

```bash
KIMI_CODE_HOME=$HOME/.kimi-code-personal kimi login
KIMI_CODE_HOME=$HOME/.kimi-code-work kimi login
```

> **提示**：如需指定区域，加上 `--region mainland-cn` 或 `--region global`。

### 3. 配置 Shell Alias

在 `~/.zshrc` 中添加：

```bash
# Kimi 多账号
alias kimi-personal='KIMI_CODE_HOME=$HOME/.kimi-code-personal kimi'
alias kimi-work='KIMI_CODE_HOME=$HOME/.kimi-code-work kimi'

# 带 yolo 模式的版本
alias kk-personal='KIMI_CODE_HOME=$HOME/.kimi-code-personal kimi -y'
alias kk-work='KIMI_CODE_HOME=$HOME/.kimi-code-work kimi -y'
```

### 4. 复用已有配置（可选）

新数据目录首次启动会生成默认配置。如果需要继承已有调优，从主目录拷贝：

```bash
TARGET=$HOME/.kimi-code-personal
cp ~/.kimi-code/config.toml "$TARGET/config.toml"
cp ~/.kimi-code/mcp.json    "$TARGET/mcp.json"   2>/dev/null
cp ~/.kimi-code/region      "$TARGET/region"      2>/dev/null
cp ~/.kimi-code/tui.toml    "$TARGET/tui.toml"    2>/dev/null
```

> **注意**：不要拷贝 `credentials/`、`oauth/`、`session_index.jsonl` 等目录——它们是账号特有的。

### 5. 关键配置项参考

`config.toml` 中常用的个性化配置：

```toml
default_model = "kimi-code/k3"          # 默认模型
default_permission_mode = "yolo"         # 权限模式：default / yolo / auto

[thinking]
enabled = true
effort = "max"                           # 思考力度：low / high / max

[loop_control]
max_attempts_per_step = 3
reserved_context_size = 50000

[background]
max_running_tasks = 4

[mcp.client]
tool_call_timeout_ms = 60000
```

### 6. 验证

```bash
# 确认各自的数据目录正确
KIMI_CODE_HOME=$HOME/.kimi-code-personal kimi doctor
KIMI_CODE_HOME=$HOME/.kimi-code-work kimi doctor
```

---

## 数据目录结构对照

### Claude Code `~/.claude[-<profile>]/`

```
├── credentials.json      # 登录凭证
├── settings.json         # 用户设置
├── projects/             # 项目级权限和设置
├── todos/                # 待办
└── statsig/              # 遥测
```

### Kimi Code `~/.kimi-code[-<profile>]/`

```
├── config.toml           # 主配置（模型、权限、hooks 等）
├── tui.toml              # TUI 外观配置
├── mcp.json              # MCP 服务器配置
├── region                # 登录区域（cn / global）
├── credentials/          # 登录凭证
│   └── kimi-code.json
├── oauth/                # OAuth 状态
├── device_id             # 设备 ID
├── session_index.jsonl   # 会话索引
├── cache/                # 缓存
├── logs/                 # 日志
├── search-index/         # 搜索索引
└── user-history/         # 输入历史
```

---

## 注意事项

1. **会话完全隔离**：不同账号的会话互不可见，`kimi -S` / `kimi -c` 只能看到当前 `KIMI_CODE_HOME` 下的会话。
2. **MCP 需各自配置**：项目级 `.mcp.json` 是共享的，但用户级 `mcp.json` 随数据目录隔离。
3. **Orca hooks**：如果使用 Orca 管理 Kimi，hooks 只会写入默认的 `~/.kimi-code/config.toml`，新账号需手动添加。
4. **二进制共享**：两个方案都只是隔离数据目录，CLI 二进制文件本身是共享的，无需重复安装。
