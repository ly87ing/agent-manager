# 故障排查手册

> 本文档提供了 Agent Manager 使用过程中常见问题的诊断步骤和解决方案。

## 目录

1. [快速开始诊断](#快速开始诊断)
2. [Agent 安装问题](#agent-安装问题)
3. [MCP 配置问题](#mcp-配置问题)
4. [Skill 链接问题](#skill-链接问题)
5. [凭证管理问题](#凭证管理问题)
6. [同步失败问题](#同步失败问题)
7. [性能问题](#性能问题)
8. [高级诊断技巧](#高级诊断技巧)
9. [紧急恢复](#紧急恢复)

---

## 快速开始诊断

当遇到问题时，请按照以下顺序进行初步诊断：

### 1. 运行系统健康检查
```bash
./manage.sh doctor
```
这将提供全系统状态概览，显示任何明显的错误或警告。
其中缺失但尚未生成的 MCP / skills / rules 目标会归入 `托管产物尚未生成` 提示项；可选组件缺失会归入 `可选工具未就绪`。真正的配置错误、依赖缺失和内容漂移仍会阻断认证。

如需供脚本或 CI 解析，请改用：
```bash
./manage.sh doctor --json
```
这会输出结构化 JSON，包含总体 `status`、错误/提示计数以及分 section 的条目列表。

如需先验证顶层命令编排本身而不触发真实安装/更新，可先运行：
```bash
make smoke
```
这会串行执行 `sync/update/plan/self-update` 的安全预演路径，并通过 `config/manager.smoke.json` 切到最小策略，避免把默认全量策略下预期存在的未物化产物混入 smoke 基线。

### 2. 检查日志输出
Agent Manager 使用标准化日志格式。最近的操作日志位于：
- `/tmp/agent_manager_sync_<user>.latest` (最近一次运行的日志指针)
- `/tmp/agent_manager_sync_<user>_<runid>.log` (按次归档的完整日志)
- 各组件特定日志（在操作过程中输出到终端）

### 3. 验证基本依赖
```bash
# 先确认基础命令存在
which python3 git

# 如当前策略启用了 brew / npm / gh provider、repo 型 tool 或需要 setup 的资源，
# 再补查对应命令
which brew npm gh

# 检查版本
python3 --version
git --version
brew --version  # 如果使用brew
npm --version   # 如果使用npm
gh --version    # 如果使用gh extension
```

### 4. 查看配置文件语法
```bash
# 验证JSON配置文件
python3 -m json.tool config/manager.json > /dev/null && echo "manager.json: 有效"
python3 -m json.tool config/agents.json > /dev/null && echo "agents.json: 有效"
python3 -m json.tool config/tools.json > /dev/null && echo "tools.json: 有效"
python3 -m json.tool config/skills.json > /dev/null && echo "skills.json: 有效"
python3 -m json.tool config/mcp.json > /dev/null && echo "mcp.json: 有效"
test -f config/rules-targets.list && echo "rules-targets.list: 存在"
test -d agent-rule/rules/src && echo "rules src: 存在"
test -d agent-rule/rules/dist && echo "rules dist: 存在"
```

---

## Agent 安装问题

### 问题：Agent 未安装或无法执行
**症状**：`manage.sh doctor` 或 `./manage.sh detect` 显示 Agent 状态为 ○ 或 未发现

#### 常见原因和解决方案：

1. **安装来源不可用**
   - 检查：`./manage.sh detect` 显示具体哪些安装来源失败
   - 解决：确保对应的包管理器已安装和可用
     - 对于 `Claude Code` 官方 native provider：确认 `curl`、`bash` 和官方安装目录在 PATH 中
     - 对于 `Codex` 的 npm provider：`npm config get prefix` 应返回可写目录
     - 对于 `Antigravity CLI` 的 native provider：确认 `curl`、`bash` 和 `~/.local/bin` 在 PATH 中
     - 对于 gh provider：`gh auth status` 检查 GitHub 认证

2. **网络或权限问题**
   - 检查：尝试手动执行安装命令
     ```bash
     # 例如，对于 Codex 官方 npm 安装
     npm install -g @openai/codex
     ```
   - 解决：解决网络连接问题或使用 sudo（如果需要且安全）

3. **冲突的安装来源**
   - 检查：`manage.sh detect` 显示多源安装冲突 (如 brew 和 npm / native 同时安装)
   - 解决：保留当前受管 provider，并移除历史或冲突来源
     ```bash
     # 例如，Claude Code 已切到 native 时，先卸载历史 brew 版本
     brew uninstall --cask claude-code
     ```

4. **路径未更新**
   - 检查：手动安装后，终端未重新加载 PATH
   - 解决：重新启动终端或执行 `hash -r` 重新加载命令路径缓存

### 问题：历史 brew 安装被判定为不合规
**症状**：`doctor` 报告 `已弃用安装来源`，或 `update` / `bootstrap` 直接失败

#### 处理建议：
处理原则：当前只维护 `Claude Code`、`Codex` 和 `Antigravity CLI`。放弃 `brew` 或不用 `brew` 不等于所有 agent 都必须 native installer；`Claude Code` 和 `Codex` 的 `npm` 固定版本、`Antigravity CLI` 的官方 native installer 是当前合规默认路径。

1. `Claude Code`
   - 手工卸载 brew 版本，再让 `agent-manager` 走官方 npm 包固定版本
   - 如果当前已经是 native 版并希望回到默认固定版本路径，先按官方方式清理 native 安装，再执行 `./manage.sh bootstrap`
2. `Codex`
   - 手工卸载 brew 版本，再执行 `npm install -g @openai/codex`
3. `Antigravity CLI`
   - 不再托管旧 Gemini CLI 的 npm/brew 来源；如需 Antigravity CLI，执行 `curl -fsSL https://antigravity.google/cli/install.sh | bash`
   - 如需阻止 `agy` 启动后触发官方后台自更新，按官方文档在 shell 环境设置 `AGY_CLI_DISABLE_AUTO_UPDATE=true`

### 问题：检测到外部安装但未纳入托管
**症状**：`doctor` 报告 `检测到外部安装，未纳入 agent-manager 托管`

#### 处理建议：
1. 确认当前 PATH 中的 `claude` / `codex` / `agy` 是否来自官方受管来源
2. 清理系统残留或自定义包装脚本，再重新执行 `./manage.sh doctor`
3. 若希望继续由 `agent-manager` 管理，请回到官方受管 provider：
   - `Claude Code`：官方 npm 包 `@anthropic-ai/claude-code`
   - `Codex`：官方 npm 包 `@openai/codex`
   - `Antigravity CLI`：官方 native installer 安装的 `agy`

### 问题：检测到官方生命周期动作缺口
**症状**：`doctor` 报告 `官方生命周期动作缺口`，或某个 Agent 条目提示 `官方当前未提供受管 uninstall 动作`

#### 处理建议：
1. 这表示某个 provider 存在官方动作缺口，不是本地配置漏写；`agent-manager` 会保持 fail-close。
2. 当前默认 provider 必须明确声明生命周期动作边界；`Antigravity CLI` 没有显式受管 update 命令，因此默认关闭 `auto_update` 并依赖 CLI 自更新。
3. 对 `Antigravity CLI`，agent-manager 不替换官方 installer 或写 shell profile；确定运行二进制版本应使用官方支持的 `AGY_CLI_DISABLE_AUTO_UPDATE=true`。
3. 如需停用存在动作缺口的 provider，请先按官方流程自行卸载，再重新运行 `./manage.sh doctor` 确认状态；`agent-manager` 不会伪造删除命令。

### 问题：Rules 修改后没有分发到目标 Agent
**症状**：已经修改 `agent-rule/rules/src/`，但目标 Agent 目录里的规则仍然是旧内容

#### 检查与解决：
1. 先重建分发产物
   ```bash
   make rules-build
   ```
2. 再执行规则同步
   ```bash
   make rules:sync
   ```
3. 如需确认产物是否存在，检查：
   ```bash
   find agent-rule/rules/dist -maxdepth 2 -type f | sort
   ```

### 问题：Agent 安装后无法正常工作
**症状**：Agent 已安装但功能异常或报错

#### 常见原因和解决方案：

1. **版本不兼容**
   - 检查：查看 Agent 版本是否符合最低要求
     ```bash
     claude --version    # Claude Code
     codex --version     # Codex
     agy --version       # Antigravity CLI
     ```
   - 解决：参考官方文档升级到兼容版本

2. **配置损坏**
   - 检查：查看 Agent 的配置文件是否为有效 JSON/TOML
     ```bash
     python3 -m json.tool ~/.claude.json > /dev/null && echo "Claude 配置有效"
     # 或对于 TOML
     python3 - <<'PY'
     import os
     try:
         import tomllib
     except ModuleNotFoundError:
         import tomli as tomllib
     with open(os.path.expanduser("~/.codex/config.toml"), "rb") as fh:
         tomllib.load(fh)
     print("Codex 配置有效")
     PY
     # Python 3.10 及以下若提示缺少 tomli，请先执行: python3 -m pip install tomli
     ```
   - 解决：备份损坏的配置并让 Agent Manager 重新生成，或手动修复

3. **权限问题**
   - 检查：配置文件或可执行文件权限是否正确
     ```bash
     ls -la ~/.claude.json
     ls -la $(which claude)
     ```
   - 解决：调整文件权限（通常配置文件应为 600，可执行文件应为 755）

---

## MCP 配置问题

### 问题：MCP 服务器无法启动或连接失败
**症状**：Agent 报告 MCP 服务器错误，如 "Failed to connect"、"Invalid configuration" 等

#### 常见原因和解决方案：

1. **配置格式错误**
   - 检查：参考 MCP-CONFIG-GUIDE.md 中的格式对比表验证配置
   - 常见错误：
     * Codex：TOML 中 JSON 字符串未用单引号包裹
     * Claude Code / Antigravity CLI：根键应为 `mcpServers`，环境变量键应为 `env`
   - 解决：根据具体 Agent 类型修正配置格式

2. **凭证未正确注入**
   - 检查：
     ```bash
     # 检查 secrets 是否存在并包含所需值
     ls -la secrets/
     cat secrets/mcp.env  # 注意：这会显示凭证！
     
     # 检查占位符是否被正确替换
     grep -r "AUGMENT_SESSION_AUTH" ~/.claude.json ~/.gemini/antigravity-cli/mcp_config.json ~/.codex/config.toml
     ```
   - 解决：
     * 确保 secrets/ 目录存在并为有效的 Git 仓库
     * 验证 secrets/mcp.env 包含所需的键值对
     * 运行 `make mcp-sync` 重新同步凭证
     * 检查日志中是否有"正在从凭证源加载"的消息

3. **命令不可用**
   - 检查：确保 auggie 命令可用并在 PATH 中
     ```bash
     which auggie
     auggie --version  # 应显示版本信息
     ```
   - 解决：安装或更新 auggie
     ```bash
     npm install -g @augmentcode/auggie
     # 或如果通过其他方式安装
     ```

4. **环境变量传递问题**
   - 检查：手动测试 MCP 服务器命令
     ```bash
     # 设置测试环境变量
     export AUGMENT_SESSION_AUTH='{"accessToken":"test","tenantURL":"test","scopes":[]}'
     auggie --mcp --mcp-auto-workspace
     # 应该看到服务器启动并等待连接（用 Ctrl+C 停止）
     ```
   - 解决：检查 Agent Manager 是否正确将凭证注入到环境变量中

### 问题：Agent 显示 MCP 服务器已连接但功能不可用
**症状**：Agent 显示 MCP 服务器已连接，但调用相关工具（如 codebase-retrieval）无效或返回错误

#### 常见原因和解决方案：

1. **服务器未正确初始化**
   - 检查：查看 MCP 服务器的完整输出（可能需要调试模式）
   - 解决：一些服务器需要特定的启动参数或环境

2. **工具名称或命名空间不匹配**
   - 检查：确认您调用的工具名称与 MCP 服务器注册的名称完全一致
   - 解决：参考 MCP 服务器文档确认正确的工具名称

3. **权限或范围不足**
   - 检查：凭证是否具有执行所需操作的必要权限
   - 解决：更新凭证以包含所需的作用域或权限

---

## Skill 链接问题

### 问题：Skill 未链接到目标 Agent
**症状**：在 Agent 的 skill/ 或 rules/ 目录中找不到预期的 skill 链接

#### 常见原因和解决方案：

1. **Skill 仓库克隆失败**
   - 检查：查看 `agent-skill/repositories/` 目录中是否存在对应的 skill 仓库缓存
     ```bash
     ls -la agent-skill/repositories/
     # 应该看到 agent-skills/ 等仓库目录
     ```
   - 解决：
     * 验证网络连接和 Git 访问权限
     * 手动尝试克隆：`git clone <skill-repo-url> agent-skill/repositories/<repo-name>`
     * 检查 Git 凭证或 SSH 密钥设置（如果是私有仓库）

2. **依赖安装失败**
   - 检查：查看 skill 同步过程中的错误日志
   - 常见问题：
     * Node.js 项目：缺少 package.json 或依赖冲突
     * Python 项目：缺少 requirements.txt 或虚拟环境问题
   - 解决：
     * 检查 skill 仓库的 README 了解特殊安装要求
     * 手动进入 skill 目录尝试安装依赖
     * 确保系统有相应的语言运行时（Node.js, Python 等）

3. **符号链接创建失败**
   - 检查：目标 Agent 的 skill/ 或 rules/ 目录权限
   - 解决：
     * 确保目标目录存在且可写
     * 对 Codex 来说，如果 skill 仓库显式限定了 `agents: ["codex"]`，首次同步会自动创建 `~/.codex/skills`
     * 其他 Agent 仍要求其目标目录父路径已存在，且对应 Agent 已被系统判定为已安装
     * 在 Windows 上，可能需要以管理员身份运行或开发者模式
     * 检查是否有现有的冲突文件或目录阻止链接创建

4. **`skills:upgrade` 没有实际升级任何仓库**
   - 检查：命令输出是否提示“未配置 `upgrade_strategy`，跳过显式升级”
   - 解决：
     * 这是信息提示，不是失败；表示当前启用的 skill 仓库没有声明可自动解析的跟踪策略
     * 只有在 `config/skills.json` 的仓库定义里配置了 `upgrade_strategy`（如 `latest-branch` / `latest-tag` / `latest-release`）时，`make skills:upgrade` 才会实际尝试升级

### 问题：链接的 skill 无法被 Agent 识别
**症状**：Skill 已链接但 Agent 未响应 skill 触发词或命令

#### 常见原因和解决方案：

1. **链接路径错误**
   - 检查：符号链接是否指向正确的 SKILL.md 文件
     ```bash
     ls -la ~/.claude/skills/  # 或对应 Agent 的 skill 目录
     # 应该看到指向正确位置的链接
     ```
   - 解决：删除错误的链接并重新运行 skill 同步

2. **Skill 未正确注册**
   - 检查：Skill 的 SKILL.md 文件是否包含正确的元数据
     ```bash
     find agent-skill/repositories/agent-skills -name SKILL.md -maxdepth 6 | head -n 1
     # 应该看到 name:, description: 等元数据
     ```
   - 解决：确保 skill 仓库包含有效的 SKILL.md 文件

3. **Agent 需要重启或重新加载**
   - 检查：某些 Agent 可能需要重新启动才能识别新技能
   - 解决：重启 Agent 客户端或重新加载配置

---

## 凭证管理问题

### 问题：凭证未被识别或认证失败
**症状**：MCP 服务器或其他集成服务报告认证错误

#### 常见原因和解决方案：

1. **secrets/ 目录未初始化**
   - 检查：`ls -la secrets/` 是否显示为 Git 仓库
   - 解决：运行 `SECRETS_REPO_URL=https://example.com/your-secrets.git ./manage.sh update`、`SECRETS_REPO_URL=https://example.com/your-secrets.git ./manage.sh bootstrap`，或执行 `./scripts/init-secrets.sh https://example.com/your-secrets.git`

2. **凭证文件格式错误**
   - 检查：`secrets/mcp.env` 是否为有效的 KEY=VALUE 格式
   - 常见错误：
     * 值中包含未转义的特殊字符
     * 使用了不支持的引号风格
     * 行格式不正确（缺少等号等）
   - 解决：参凭证管理指南修正文件格式

3. **占位符未正确替换**
   - 检查：目标配置文件中是否仍存在 `${VAR}` 形式的占位符
   - 解决：
     * 确认 secrets/mcp.env 中包含对应的键
     * 检查 Agent Manager 日志中是否有"正在从凭证源加载"的消息
     * 手动测试占位符替换：`envsubst < config/mcp.json` （如果安装了 gettext）

4. **凭证已过期或被撤销**
   - 检查：与服务提供商确认凭证状态
   - 解决：获取新凭证并更新 secrets/mcp.env

### 问题：凭证泄露风险
**症状**：怀疑凭证已暴露或凭证仓库意外公开

#### 应急措施：
1. **立即行动**：
   - 撤销或轮换所有可能暴露的凭证
   - 检查 Git 历史：`cd secrets && git log -p` 查看是否曾提交凭证到公共仓库
   - 检查日志文件以确认凭证是否被意外输出

2. **长期预防**：
   - 使用 GitHub 的 secret scanning 或 GitLab 的秘密检测
   - 在 CI/CD 中添加凭证泄露检查步骤
   - 教育团队成员关于凭证安全的最佳实践
   - 考虑使用专门的密钥管理服务（如 HashiCorp Vault, AWS Secrets Manager）

---

## 同步失败问题

### 问题：make update / make bootstrap 或对应的 manage.sh 命令执行失败
**症状**：同步过程在某个阶段中止并返回错误码

#### 常见原因和解决方案：

1. **特定组件失败**
   - 检查：错误消息通常会指示失败的组件（如 "FAILED: agents"、"FAILED: mcp" 等）
   - 解决：针对性地运行对应的同步命令以获取更详细的信息
     ```bash
     # 例如，如果是 MCP 失败
     cd agent-mcp && python3 sync_mcp.py --config ../config/manager.json --def-file ../config/mcp.json
     ```

2. **网络或下载问题**
   - 检查：尝试手动下载或克隆失败的资源
   - 解决：
     * 使用 `--offline` 标志跳过网络操作进行测试
     * 检查代理设置或防火墙规则
     * 尝试使用不同的网络或VPN

3. **权限问题**
   - 检查：尝试操作的目录是否可写
   - 解决：确保对所有目标目录（如 ~/.claude/, ~/.gemini/ 等）有写入权限

4. **依赖冲突或缺失**
   - 检查：查看具体组件的错误日志寻找依赖相关错误
   - 解决：
     * 安装缺失的系统依赖（如特定版本的 Node.js, Python 等）
     * 清除可能的缓存或临时文件后重试

### 问题：同步后配置未按预期更改
**症状**：运行同步后，目标配置文件内容未达到预期状态

#### 常见原因和解决方案：

1. **被排除的配置**
   - 检查：配置是否被策略中的 `enabled: false` 排除
   - 解决：在 manager.json 中将相应项设置为 `enabled: true`

2. **冲突的手动修改**
   - 检查：是否在同步后手动修改了配置文件
   - 解决：Agent Manager 设计为覆盖手动修改；如需保留自定义内容，请使用官方扩展机制

3. **同步未实际运行**
   - 检查：确认同步命令确实执行并完成了目标操作
   - 解决：查看同步日志确认所有步骤都已处理

---

## 性能问题

### 问题：同步过程异常缓慢
**症状**：`make update` / `make bootstrap` 执行时间异常长（正常应在几分钟内完成）

#### 常见原因和解决方案：

1. **频繁的包管理器查询**
   - 检查：是否反复运行 `brew list` 或类似命令
   - 解决：Agent Manager 已实现全局状态缓存以避免此问题；如仍然缓慢，检查缓存机制是否正常工作

2. **大型仓库克隆**
   - 检查：某些 skill 或 MCP 仓库可能非常大
   - 解决：
     * 考虑使用 `--depth 1` 进行浅克隆（如果仓库支持）
     * 检查是否真的需要完整历史记录
     * 对于非常大的仓库，评估是否所有功能都必要

3. **不必要的依赖安装**
   - 检查：是否在每次同步时都重新安装依赖
   - 解决：Agent Manager 仅在依赖文件变更时重新安装；如频繁安装，检查是否有文件被误认为变更

4. **I/O 瓶颈**
   - 检查：磁盘速度或网络速度限制
   - 解决：
     * 使用更快的存储介质（SSD 而非 HDD）
     * 改善网络连接
     * 考虑将 secrets/ 等频繁访问的目录放在更快的存储上

### 问题：Agent 响应迟缓
**症状**：Agent 在使用 MCP 或 skill 时响应变慢

#### 常见原因和解决方案：

1. **MCP 服务器性能**
   - 检查：auggie 或其他 MCP 服务器的响应时间
   - 解决：
     * 优化服务器查询（如减少 codebase-retrieval 范围）
     * 检查服务器日志寻找性能瓶颈
     * 考虑升级到更高性能的服务器版本

2. **Skill 执行开销**
   - 检查：特定 skill 的执行是否涉及重计算或大量 I/O
   - 解决：
     * 使用 skill 提供的缓存机制（如果有）
     * 优化 skill 配置以减少不必要的工作
     * 检查是否有更轻量级的替代方案

3. **资源竞争**
   - 检查：系统是否在进行其他资源密集型任务
   - 解决：调度同步和重任务以避免峰值冲突

---

## 高级诊断技巧

### 启用调试日志
```bash
# 临时启用调试级别日志（所有 Python 同步引擎都会读取 LOG_LEVEL）
LOG_LEVEL=DEBUG ./manage.sh bootstrap

# 也可以只调试某个子引擎
LOG_LEVEL=DEBUG python3 agent-mcp/sync_mcp.py --config config/manager.json --def-file config/mcp.json --dry-run
```

### 检查环境变量注入
```bash
# 在同步过程中打印环境变量替换情况
# 修改 sync_mcp.py 中的 logging 级别或添加调试打印
```

### 验证文件原子写入
```bash
# 在目标配置文件所在目录中观察临时文件
watch -n 0.1 "ls -la ~/.claude/ | grep tmp"
# 同步过程中应该看到临时文件出现和消失
```

### 模拟同步而不实际修改
```bash
# 使用 --dry-run 标志预览将要执行的操作
./manage.sh plan
cd agent-mcp && python3 sync_mcp.py --config ../config/manager.json --def-file ../config/mcp.json --dry-run
# 等效的 dry-run 标志适用于所有同步引擎
```

### 检查 Git 仓库状态
```bash
# 检查所有相关仓库的状态
find agent-* -name ".git" -type d | while read gitdir; do
  echo "=== $(dirname $gitdir) ==="
  (cd $(dirname $gitdir) && git status --short && git log --oneline -3)
  echo
done
```

### 验证符号链接完整性
```bash
# 检查所有技能链接是否有效
find ~/.claude/skills ~/.gemini/antigravity-cli/plugins/agent-manager/skills ~/.codex/skills -type l 2>/dev/null | while read link; do
  if [ -e "$link" ]; then
    echo "OK: $link -> $(readlink $link)"
  else
    echo "BROKEN: $link -> $(readlink $link)"
  fi
done
```

---

## 紧急恢复

### 完全重置 Agent Manager 状态
> ⚠️ 警告：此操作将删除所有 Agent Manager 管理的配置和链接！
> 仅在其他方法失败时使用，并确保已备份重要自定义配置。

```bash
# 1. 停止所有正在运行的 Agent 实例
# 2. 删除管理的配置和链接
rm -rf ~/.claude.json ~/.gemini/antigravity-cli/mcp_config.json ~/.codex/config.toml
rm -rf ~/.claude/skills/* ~/.gemini/antigravity-cli/plugins/agent-manager/skills/* ~/.codex/skills/*

# 3. 重新从 Agent Manager 同步
./manage.sh bootstrap

# 4. 如需恢复 secrets，重新初始化
./scripts/init-secrets.sh https://example.com/your-secrets.git
```

### 恢复到已知良好状态
如果您定期备份配置（强烈推荐）：

```bash
# 假设您有备份目录 ~/agent-manager-backup/
# 1. 停止所有 Agent 实例
# 2. 恢复配置
cp -r ~/agent-manager-backup/claude.json ~/.claude.json
cp -r ~/agent-manager-backup/antigravity/mcp_config.json ~/.gemini/antigravity-cli/mcp_config.json
# ... 依次恢复所有配置
# 3. 恢复技能链接（如果备份了）
cp -r ~/agent-manager-backup/claude/skills/* ~/.claude/skills/
# ... 依次恢复所有技能目录
# 4. 重新启动 Agent
```

### 联系支持
在寻求社区或官方支持之前，请收集以下信息：
1. 完整的错误消息和堆栈跟踪（如果有）
2. 运行 `./manage.sh doctor` 的完整输出
3. 相关配置文件（移除凭证后）：
   ```bash
   # 创建无凭证的配置副本用于分享
   cp config/manager.json /tmp/manager-safe.json
   # 手动编辑移除敏感信息，或使用工具自动处理
   ```
4. 您的操作系统和环境详细信息
5. 问题发生的确切步骤（以便重现）

---

## 常见错误消息参考

### Agent Manager 特定错误
| 错误消息 | 可能原因 | 解决方案 |
|----------|----------|----------|
| `ERROR: Agent lifecycle failed` | Agent 安装/更新/卸载过程中出错 | 检查特定 Agent 的安装日志，验证包管理器可用性 |
| `ERROR: MCP failed` | MCP 配置注入失败 | 检查 MCP-CONFIG-GUIDE.md，验证凭证和格式 |
| `ERROR: Skills failed` | Skill 同步或链接失败 | 检查 `agent-skill/repositories/` 仓库缓存和同步日志，验证 Git 访问和依赖 |
| `ERROR: Tools failed` | 工具安装失败 | 检查工具特定的安装要求和网络连接 |
| `ERROR: Rules failed` | 规则同步失败 | 检查 agent-rule/ 目录和 Git 访问权限 |
| `Found multiple matches for oldString` | 配置替换时字符串匹配不唯一 | 检查正在替换的配置内容，使匹配更具体 |
| `oldString not found in content` | 尝试替换不存在的内容 | 检查目标配置是否已被其他进程修改或格式已变更 |

### 常见 Agent 错误
| 错误消息 | 可能原因 | 解决方案 |
|----------|----------|----------|
| `Unexpected key or value` (Codex) | TOML 格式错误，通常是 JSON 字符串未正确转义 | 使用单引号包裹包含双引号的值：`KEY = '{"json":"value"}'` |
| `Disconnected` (Antigravity CLI) | 通常是正常的按需启动行为 | 实际使用 MCP 功能时服务器会自动启动；使用 `/mcp` 检查 |
| `Command not found: auggie` | auggie 未安装或不在 PATH 中 | 安装：`npm install -g @augmentcode/auggie` 或检查安装路径 |
| `Authentication failed` | 凭证无效或过期 | 更新 secrets/mcp.env 中的凭证并重新同步 |

---

## 结论

本故障排查手册涵盖了 Agent Manager 使用过程中遇到的最常见问题。通过遵循此处概述的诊断步骤和解决方案，您应该能够解决大多数问题。

记住：
1. 总是从 `./manage.sh doctor` 开始进行一般诊断
2. 检查日志以获取具体错误信息
3. 逐个组件隔离问题
4. 如不确定，使用 `./manage.sh plan` 或 `--dry-run` 先预览操作
5. 定期备份重要配置和凭证
6. 保持系统和依赖的更新

对于本手册未涵盖的问题，请参考：
- [Agent Manager 架构说明](./ARCHITECTURE.md)
- [凭证管理指南](./SECRETS-MANAGEMENT.md)
- [MCP 配置完全指南](./MCP-CONFIG-GUIDE.md)
- 官方 Agent 文档
- 相关技能和工具的文档

如果问题仍然存在，请收集详细信息并寻求社区或官方支持。
