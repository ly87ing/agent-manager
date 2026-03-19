# Agent Manager 架构说明

> 本文档详细说明了 Agent Manager 系统的架构设计和核心组件。

## 目录

1. [系统概览](#系统概览)
2. [核心组件](#核心组件)
3. [数据流设计](#数据流设计)
4. [安全机制](#安全机制)
5. [扩展性设计](#扩展性设计)
6. [部署架构](#部署架构)

---

## 系统概览

Agent Manager 是一个统一的 AI 助手环境管理平台，旨在通过单一入口管理多种 AI 编码助手的完整生命周期。系统采用模块化设计，将策略意愿与具体实现分离，确保配置的精准与安全。

核心设计原则：
- **策略与定义分离**：管理用户意愿（manager.json）与技术细节（agents.json等）分离
- **原子化操作**：所有文件写入采用临时文件原子替换模式
- **显式运行时契约**：`dry-run` / `offline` / 锁 / 日志行为必须由 `manage.sh` 统一解释，不能由各同步器各自猜测
- **差异审计**：实时监控物理文件与定义仓库的内容偏离
- **高性能探测**：通过全局状态缓存避免频繁I/O操作

---

## 核心组件

### 1. 策略层 (Policy Layer)
**文件位置**: `config/manager.json`

用户决策入口，定义：
- `enabled`: 是否启用该Agent/Tool/Skill/MCP
- `auto_install`: 是否自动安装
- `auto_update`: 是否自动更新
- `preferred_provider`: 首选安装来源
- `agents` 键使用稳定的机器可读 ID（如 `claude-code`, `cursor`），而不是面向用户的展示名

### 2. 定义层 (Definition Layer)
**文件位置**: `config/*.json`

保存官方指令细节，支持Policy的强制覆盖：
- `agents.json`: Agent安装源和卸载命令
- `tools.json`: 工具安装和设置指令
- `skills.json`: Skill仓库地址和Agent映射
- `mcp.json`: MCP服务器定义和Agent路径映射
- Agent 定义中的 `display_name` 仅用于展示和日志；跨配置引用统一使用稳定 ID
- `agents.json` 中的 `providers` 只声明 agent-manager 可托管的安装源；`system` 只是运行时探测信号，不是可配置的 provider key
- `preferred_provider` 必须指向 `providers` 中真实存在的键，否则生命周期同步应直接失败
- `skills.json` 中的 skill 仓库支持 `commit` / `ref` 版本约束；技能名来自仓库内 `SKILL.md` frontmatter 的 `name`

### 3. 同步引擎 (Sync Engines)
**文件位置**: 各agent-*目录下的sync_*.py脚本

负责将策略转换为实际操作：
- `agent-lifecycle/sync_agents.py`: Agent生命周期管理
- `agent-tool/sync_tools.py`: 工具链管理
- `agent-skill/link_skills.py`: Skill链接管理
- `agent-mcp/sync_mcp.py`: MCP配置注入

其中 `agent-lifecycle/sync_agents.py` 需要明确区分三种状态：
- **managed provider**：定义文件中声明且 agent-manager 知道如何 install/uninstall/upgrade 的来源
- **system-only detect**：只是在 PATH 或 App 目录中探测到二进制/应用存在，但不代表 agent-manager 可以安全接管
- **unsupported provider**：运行时探测到了来源，但定义里没有声明；这类情况必须显式报错，不能静默继续

### 4. 审计与验证 (Audit & Verification)
**文件位置**: `manage.sh` 中的doctor_check函数

提供全系统健康检查：
- 配置语法验证
- 一致性审计（文件内容与目录结构漂移检测）
- 环境依赖检查
- 安全风险扫描

---

## 数据流设计

### 配置同步流程
```mermaid
graph TD
    A[用户修改 manager.json] --> B[同步引擎读取策略]
    B --> C[读取对应的 definition.json]
    C --> D[检测当前系统状态]
    D --> E{状态是否需要变更?}
    E -->|是| F[执行安装/更新/卸载]
    E -->|否| G[保持当前状态]
    F --> H[原子写入配置文件]
    H --> I[记录 _MANAGED_BY 标记]
    I --> J[完成同步]
```

### 凭证管理流程
```mermaid
graph TD
    A[secrets/ 目录检查] --> B{是否存在?}
    B -->|否| C[自动克隆 secrets 仓库]
    B -->|是| D[更新凭证]
    C --> E[读取 secrets/mcp.env]
    D --> E
    E --> F[替换配置中的 ${VAR} 占位符]
    F --> G[生成 Agent 特定配置]
    G --> H[原子写入目标配置文件]
```

### 管理入口运行时契约

`manage.sh` 是唯一的顶层编排入口，负责统一解释全局 flag 和运行时状态：

- **原子锁**：需要写入或同步的命令会先抢占 `/tmp/agent_manager_<user>.lock.d`，并在其中写入持有者 PID；兼容保留 `/tmp/agent_manager_<user>.lock` 作为元数据。`unlock` 会同时清理目录锁和遗留锁文件。
- **按次日志**：每次运行写入独立日志 `/tmp/agent_manager_sync_<user>_<runid>.log`，并更新 `/tmp/agent_manager_sync_<user>.latest` 指针；`logs` 读取 latest，`clean-logs` 清理 latest 与全部历史运行日志。
- **`--dry-run`**：只输出计划动作，跳过 secrets 初始化、仓库 clone/pull、依赖安装、构建，以及 `cleanup` / `unlock` / `backups:restore` / `clean-logs` 这类会修改本地状态的维护动作；允许保留只读探测和预览日志。若与 `self-update` 组合，只预览仓库自更新动作。
- **`--offline`**：禁止任何远程拉取、下载和安装。生命周期、tools、skills、MCP 同步器都只能复用本地已存在仓库/配置；如果本地前置资源缺失，应显式报错而不是偷偷联网。
- **命令分层**：`sync` 只做本地配置收敛；`update` 会补齐并刷新非 Agent 安装类资源，并只更新已安装 Agent；`bootstrap` / `all` 才负责缺失 Agent 的安装；`self-update` 只更新 agent-manager 仓库自身。

---

## 安全机制

### 1. 原子化写入 (Atomic Write)
所有配置文件写入采用以下步骤：
1. 创建临时文件 (.tmp_* 前缀)
2. 写入数据到临时文件
3. 验证写入成功
4. 原子替换目标文件
5. 设置适当文件权限 (通常是 0o600)

### 2. 凭证保护
- 凭证存储在独立的私有 git 仓库 (secrets/)
- 永不写入主仓库或日志
- 使用环境变量占位符替换
- 支持凭证轮换和审计
- `--offline` 下不会同步 secrets 仓库，只会消费本地已有 secrets 内容

### 3. 权限最小化
- 配置文件默认权限为 600 (仅所有者可读写)
- 临时文件同样采用严格权限
- 凭证仓库强烈建议设为私有

---

## 扩展性设计

### 添加新Agent
1. 在 `config/agents.json` 中添加Agent定义
   - 键名必须是稳定 ID（例如 `github-copilot`）
   - `display_name` 用于 CLI/UI 展示
   - bin: 可执行文件名称
   - preferred_provider: 首选安装来源
   - providers: 各安装来源的install/uninstall/upgrade命令
2. 在 `config/manager.json` 中添加策略条目
3. 如需MCP支持，在 `config/mcp.json` 的agent_paths中添加映射
4. 如需Skill支持，在 `config/skills.json` 的agents中添加映射

### 添加新MCP服务器
1. 在 `config/mcp.json` 的servers中添加定义
   - command: 执行命令
   - args: 参数列表
   - env: 环境变量映射（支持${VAR}占位符）
   - agents: 限定生效的Agent列表（null表示全部）
2. 系统将自动处理不同Agent的格式差异

### 添加新Skill
1. 在 `config/skills.json` 的repositories中添加Git仓库
   - url: 仓库地址
   - commit / ref: 可选版本约束；`commit` 优先级高于 `ref`
   - agents: 限定生效的Agent列表（null表示全部）
2. 仓库会被缓存到 `agent-skill/repositories/<repo-name>/`
3. 只有带 `.agent-manager-owned` 标记的托管缓存目录才会被自动清理
4. 技能目录必须包含 `SKILL.md`，并在 frontmatter 中声明 `name`
5. `auto_update: false` 时系统会复用本地缓存但跳过 `git pull`；`--offline` 则禁止下载和远程更新

---

## 部署架构

### 依赖要求
- Python 3.6+
- Git
- 根据所管理的Agent：brew, npm, gh等

### 目录结构
```
agent-manager/
├── config/                    # 配置文件
│   ├── manager.json          # 策略层
│   ├── agents.json           # Agent定义
│   ├── tools.json            # 工具定义
│   ├── skills.json           # Skill定义
│   └── mcp.json              # MCP定义
├── agent-lifecycle/          # Agent生命周期管理
├── agent-tool/               # 工具管理
├── agent-skill/              # Skill管理
│   └── repositories/         # Skill 仓库托管缓存（owner-marked）
├── agent-mcp/                # MCP管理
├── agent-rule/               # 规则管理
├── secrets/                  # 凭证仓库（git子模块或独立仓库）
├── docs/                     # 文档
└── manage.sh                 # 主管理脚本
```

### 工作流程
1. 用户通过修改 `config/manager.json` 表达意愿
2. 选择入口：`make sync` 做本地配置收敛，`make update` 刷新远端资源，`make bootstrap` 做全量安装、更新与同步；`./manage.sh all` 仅作为兼容别名保留
3. `manage.sh` 先解析 `dry-run` / `offline`，获取原子锁，并建立本次运行专属日志
4. 各同步引擎读取策略和定义，检测当前状态
5. 根据运行模式执行安装、更新或卸载操作
6. 生成Agent特定的配置文件并原子写入
7. 记录管理标记 (_MANAGED_BY = "agent-manager") 以便审计
8. 运行健康检查验证同步结果

---

## 设计决策说明

### 为什么选择JSON作为主要配置格式？
- 广泛支持：所有目标Agent都支持JSON配置
- 人类可读：便于手动检查和编辑
- 机器友好：易于程序解析和生成
- 生态成熟：丰富的工具和库支持

### 为什么保留TOML支持（用于Codex）？
- 某些Agent（如Codex）原生使用TOML
- 为了最大兼容性，系统必须支持目标Agent的原生格式
- 通过特殊处理确保TOML中的特殊字符正确转义

### 为什么 Rules 同时支持复制和符号链接，并默认使用复制？
- 默认复制更稳妥：目标目录可独立审计、备份和回滚，不依赖源仓库持续存在
- 目录型规则需要收敛同步：复制模式可以在分发时删除陈旧文件，避免模块残留
- 符号链接仍然保留给显式选择的高级用法，用于节省空间或实时跟随源内容
- `manage.sh doctor` 会按文件内容和目录结构审计规则目标，而不是只对单文件做 MD5 对比

---

## 未来改进方向

1. **增强插件机制**：支持更灵活的同步引擎插件
2. **图形化界面**：基于Web的配置管理界面
3. **高级审计**：更详细的变更历史和影响分析
4. **多环境支持**：开发/测试/生产环境隔离
5. **性能优化**：进一步减少同步时间和资源占用
