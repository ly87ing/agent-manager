# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 常用命令

```bash
# 运行所有测试（无需额外依赖，使用标准库 unittest）
python3 -m unittest discover -s tests -p 'test*.py' -v

# 运行单个测试文件
python3 -m unittest tests.test_manage -v

# 运行单个测试用例
python3 -m unittest tests.test_manage.TestClassName.test_method_name

# 跨平台 smoke 验证（安全预演，不产生副作用）
make smoke
# 或
./scripts/ci-smoke.sh

# 仅本地配置收敛（不联网、不安装）
make sync

# 预演完整 bootstrap（dry-run 模式）
make plan

# 全系统健康检查
make doctor
```

## 架构概览

### 核心设计原则

- **策略与定义分离**：`config/manager.json` 提供仓库基线，`config/manager.local.json` 提供机器本地覆盖；`config/agents.json`、`tools.json`、`skills.json`、`mcp.json` 保存技术细节
- **原子化写入**：所有配置文件写入使用临时文件原子替换模式；目录型 rules 使用收敛式同步
- **显式运行时契约**：`dry-run` / `offline` / 锁 / 日志行为由 `manage.sh` 统一解释，各同步器不自行猜测
- **`_MANAGED_BY` 标记**：所有托管写入的配置都注入此标记，用于审计追踪

### 入口与编排

`manage.sh` 是唯一顶层编排入口：
1. 解析 `--dry-run` / `--offline` 全局 flag
2. 抢占原子锁 `/tmp/agent_manager_<user>.lock.d`
3. 建立本次运行专属日志 `/tmp/agent_manager_sync_<user>_<runid>.log`
4. 调度各 Python 同步引擎

### 同步引擎（各 `agent-*/` 目录）

| 引擎 | 脚本 | 职责 |
|------|------|------|
| agent-lifecycle | `sync_agents.py` | Agent 安装 / 更新 / 卸载生命周期 |
| agent-tool | `sync_tools.py` | 工具链管理 |
| agent-skill | `link_skills.py` | Skill 仓库克隆与链接 |
| agent-mcp | `sync_mcp.py` | MCP 配置注入到各 Agent 配置文件 |
| agent-rule | `scripts/sync-agent-rules.sh` | Rules 文件分发到各 Agent 规则目录（shell 实现） |
| agent-terminal | `sync_terminal.py` | 终端配置模板渲染与原子写入（zshrc 等） |

所有 Python 同步引擎都必须从 `src.runtime_logging` 导入 `configure_logging()` 以共享统一日志级别控制。`agent-rule` 是唯一的 shell 实现，由 `manage.sh` 直接调用。

### `src/` 共享模块

- `runtime_logging.py`：统一日志配置，支持 `LOG_LEVEL=DEBUG` 环境变量
- `json_config.py`：原子写入、JSON 读写工具
- `agent_ids.py`：稳定的机器可读 Agent ID 常量
- `interactive.py`：交互式 UI 组件
- `manage_runtime_probe.py`：运行时环境探测（PATH 扫描、`.app` 目录兜底）
- `template_utils.py`：模板渲染（`${VAR}` 占位符替换）与 env 文件加载

### 配置层级

```
config/manager.json       ← 策略层基线：仓库默认意愿（enabled / auto_install / preferred_provider）
config/manager.local.json ← 策略层覆盖：本机私有偏好；运行时深度合并，UI 默认写入这里
config/agents.json        ← 定义层：安装源 providers，必须与 manager.json 中的 preferred_provider 对应
config/mcp.json           ← MCP 服务器定义，secrets/mcp.env 中的 ${VAR} 在注入时替换
config/skills.json        ← Skill 仓库 URL + commit/ref 版本约束 + Agent 映射
config/tools.json         ← 工具安装指令（git-repo 型：含 url + setup；npm 型：含 install）
config/terminal.json      ← 终端配置模板目标清单（模板路径、写入路径、权限、前置依赖）
config/rules-targets.list ← Rules 分发清单，格式：name|group|source|target
```

`preferred_provider` 必须指向 `agents.json` 中 `providers` 的真实键，否则同步直接失败（不静默降级）。`terminal.enabled` 默认为 `false`，主流程不会隐式改写终端配置。

**Codex 的特殊定位**：`codex` 作为 MCP、Rules 与 Skills 的分发目标（在 `mcp.json`、`rules-targets.list`、`skills.json` 中有条目），但不纳入生命周期管理（`agents.json` 和 `manager.json` 中无 codex）。这是有意设计。

### Agent 配置文件路径（官方规范）

| Agent | 配置文件 | MCP 键名 |
|-------|---------|----------|
| Claude Code | `~/.claude.json` | `mcpServers` |
| Gemini CLI | `~/.gemini/settings.json` | `mcpServers` |
| Cursor | `~/.cursor/mcp.json` | `mcpServers` |
| GitHub Copilot | `~/.copilot/mcp-config.json` | `mcpServers` |
| OpenCode | `~/.config/opencode/opencode.json` | `mcp` |
| Codex | `~/.codex/config.toml` | `mcp_servers` (TOML) |

### 测试结构

`tests/` 目录包含集成测试和契约测试，直接读取项目文件验证行为（非 mock）。关键测试文件：
- `test_project_contracts.py`：验证文档与代码行为一致性的契约测试
- `test_manage.py`：`manage.sh` 行为测试
- `test_manage_runtime_probe.py`：环境探测逻辑测试
- `tests/fixtures/`：隔离的测试夹具

## 调试

```bash
# 开启 DEBUG 日志
LOG_LEVEL=DEBUG ./manage.sh bootstrap

# 强制解除残留锁（进程异常退出后）
make unlock
```

锁文件位于 `/tmp/agent_manager_<user>.lock.d`，日志位于 `/tmp/agent_manager_sync_<user>_<runid>.log`。
