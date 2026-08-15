# 凭证管理指南

> 本文档详细说明了 Agent Manager 中的凭证管理机制、最佳实践和操作指南。

## 目录

1. [凭证管理概述](#凭证管理概述)
2. [凭证存储架构](#凭证存储架构)
3. [支持的凭证类型](#支持的凭证类型)
4. [凭证工作流程](#凭证工作流程)
5. [最佳实践](#最佳实践)
6. [常见操作](#常见操作)
7. [故障排查](#故障排查)

---

## 凭证管理概述

Agent Manager 采用**凭证外置**的设计理念，将敏感信息（如API密钥、访问令牌等）与主配置分离存储。这种设计带来以下优势：

- **安全性提升**：凭证不暴露在主仓库或日志中
- **版本控制**：凭证可以独立版本化和审计
- **权限分离**：可以独立管理凭证仓库的访问权限
- **轮换便利**：凭证更新不影响主配置

核心原则：
1. 主仓库永不包含真实凭证
2. 凭证通过环境变量安全注入到目标配置中
3. 凭证仓库强烈建议设为私有
4. 支持凭证的自动同步和手动更新
5. `Tailscale` / `SSH` / 远程访问相关的真实网络身份信息也属于敏感信息，按 secrets 管理

---

## 凭证存储架构

### 目录结构
```
secrets/
├── mcp.env                 # 主要的MCP凭证文件
├── .git/                   # 凭证仓库的Git元数据
└── 其他环境特定的.env文件
```

### 文件格式
凭证文件采用标准的 `KEY=VALUE` 格式，每行一个键值对：
```bash
# secrets/mcp.env
GROK_API_KEY='sk-abc123...'
ANOTHER_API_KEY="sk-abc123..."
```

注意：
- 值可以使用单引号、双引号或无引号（取决于内容）
- 包含特殊字符（如JSON）时建议使用单引号避免转义问题
- 注释行以 `#` 开头
- 空行会被忽略

### Git仓库特性
凭证存储在独立的Git仓库中，提供：
- **变更历史**：所有凭证变更可追溯
- **分支支持**：可为不同环境维护不同分支
- **访问控制**：通过Git服务器的权限管理
- **离线支持**：可在本地拥有完整历史记录

---

## 支持的凭证类型

目前 Agent Manager 主要处理以下类型的凭证：

### 1. MCP服务器凭证
用于受管 MCP 服务器的认证：
- `GROK_API_URL`: grok-search MCP 使用的 Grok API 入口
- `GROK_API_KEY`: grok-search MCP 使用的 Grok API Key
- `GROK_MODEL`: grok-search MCP 默认模型，例如 `grok-4-fast`
- `TAVILY_ENABLED`: grok-search MCP 是否启用 Tavily provider，例如 `true`
- `TAVILY_API_KEY`: grok-search MCP 使用的 Tavily API Key
- `TAVILY_API_URL`: grok-search MCP 使用的 Tavily API 端点，例如 `https://api.tavily.com`
- `FIRECRAWL_API_KEY`: grok-search MCP 使用的 Firecrawl API Key
- `FIRECRAWL_API_URL`: grok-search MCP 使用的 Firecrawl API 端点，例如 `https://api.firecrawl.dev/v2`

### 2. Claude Code 凭证
Claude Code 凭证不再由 agent-manager 注入，也不再维护 `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_BASE_URL` 的环境变量同步链路。Claude Code 的 provider、key 和 `~/.claude/settings.json` 由 cc-switch 维护，避免 agent-manager 与 cc-switch 对同一组 env key 反复覆盖。

仓库仍保留 `config/claude-env.json` 作为公开 rule：它只声明 Claude Code provider env key 的占位符和 settings 路径，不包含真实值，也不会被 agent-manager 写入用户级 Claude Code 配置。

### 3. 其他服务凭证
根据集成的服务而定义：
- GitHub Personal Access Token (用于某些扩展)
- API keys for various AI providers
- Database connection strings
- 其他第三方服务凭证

### 4. 远程访问与网络身份信息
以下信息虽然不一定是传统意义上的 API key，但在本项目中同样按敏感信息处理：
- `Tailscale` 设备名、Tailnet DNS name、完整 `.ts.net` 域名
- `Tailscale` IPv4 地址、ACL / grants 中使用的真实主机映射
- `SSH` Host 配置中的真实 `Host`、`HostName`、`User`
- `authorized_keys` 中的真实公钥内容、key label、可识别设备身份的注释
- 可推导私网拓扑、远程入口和设备对应关系的组合信息

这些值只允许存放在外部私有 `secrets/` 仓库中，不允许出现在 `agent-manager` 的代码、文档、示例、计划、HTML 页面或 README 中。当前仓库如需举例，统一使用 `YOUR_*` 占位符。

### 凭�命名约定
建议使用以下命名约定：
- 全大写，下划线分隔
- 前缀表示服务来源：`GROK_`, `TAVILY_`, `FIRECRAWL_` 等
- 描述性后缀：`_TOKEN`, `_KEY`, `_URL`, `_AUTH`

---

## 凭证工作流程

### 自动化流程（推荐）
```mermaid
graph TD
    A[运行 make bump / make update / make init] --> B{secrets/ 目录存在吗?}
    B -->|否| C[从 SECRETS_REPO_URL 或脚本参数读取私有仓库地址]
    B -->|是| D[更新现有 secrets 仓库]
    C --> E[读取 secrets/mcp.env]
    D --> E
    E --> F[解析 KEY=VALUE 行]
    F --> G[替换配置文件中的 ${VAR} 占位符]
    G --> H[生成 Agent 特定的配置]
    H --> I[原子写入目标配置文件]
    I --> J[完成同步]
```

### 手动凭证更新流程
```mermaid
graph TD
    A[获取新凭证] --> B[编辑 secrets/mcp.env]
    B --> C[git add mcp.env]
    C --> D[git commit -m "描述更新内容"]
    D --> E[git push origin main]
    E --> F[运行 make mcp-apply 或 ./manage.sh mcp:apply]
    F --> G[系统重新读取凭证并更新配置]
```

---

## 最佳实践

### 1. 仓库安全
- **必设私有**：凭证仓库必须设置为私有仓库
- **限制访问**：只授予必要人员访问权限
- **定期审计**：检查访问日志和权限设置
- **使用机器用户**：对于自动化系统，使用专门的机器用户而非个人账户

### 2. 凭证管理
- **定期轮换**：建议每90天轮换一次重要凭证
- **最小权限原则**：凭证应具备完成所需任务的最小权限
- **环境隔离**：生产、测试、开发环境使用不同凭证
- **避免硬编码**：永不在代码或配置中硬编码凭证
- **远程访问信息同级保护**：真实 `Tailscale` / `SSH` / 远程 Host 信息视同凭证，不写入主仓库

### 3. 文件管理
- **使用单引号**：对于包含JSON或特殊字符的值，使用单引号避免转义问题
- **一致性格式**：保持键值对格式的一致性
- **适当注释**：为重要凭证添加说明注释
- **定义清理**：定期移除不再使用的凭证

### 4. 操作规范
- **验证变更**：凭证更新后，验证相关服务仍能正常工作
- **记录变更**：在提交消息中说明更新原因（但不包含实际凭辑值）
- **备份策略**：考虑凭证仓库的备份策略
- **紧急撤销**：了解如何快速撤销泄露的凭证

---

## 常见操作

### 初始化凭证仓库
```bash
# 方法1：通过 make 命令自动初始化（推荐）
SECRETS_REPO_URL=https://example.com/your-secrets.git make bump
# 系统会检测secrets/目录是否存在并自动克隆

# 方法2：手动运行初始化脚本
./scripts/init-secrets.sh https://example.com/your-secrets.git

# 方法3：直接克隆
git clone https://example.com/your-secrets.git secrets
```

### 更新凭证
```bash
# 进入secrets目录
cd secrets

# 编辑凭证文件
vim mcp.env
# 或使用echo更新特定值
echo 'GROK_API_KEY='\''new_value'\'' > mcp.env

# 提交变更
git add mcp.env
git commit -m "Update MCP credentials for security rotation"
git push origin main

# 同步到Agent配置
cd ..
make mcp-apply
```

### 查看凭证状态
```bash
# 检查secrets目录状态
ls -la secrets/

# 查看凭证文件内容（注意：这会显示凭谨！仅在安全环境中操作）
cat secrets/mcp.env

# 检查Git状态
cd secrets && git status && git log --oneline -5
```

### 凭证轮换流程
```bash
# 1. 在服务提供商处生成新凭证
# 2. 更新secrets/mcp.env
cd secrets
vim mcp.env  # 替换为新凭证
# 3. 提交变更
git add mcp.env
git commit -m "Rotate credentials - $(date +%Y-%m-%d)"
git push origin main
# 4. 同步更新
cd ..
make mcp-apply
# 5. 验证服务正常工作
# 6. 如需要，可保留旧凭证一段时间以确保平稳过渡
```

---

## 故障排查

### 问题：凭证未被识别
**症状**：MCP服务器认证失败，尽管凭证文件存在

**检查步骤**：
1. 验证secrets/目录是否存在并为Git仓库
2. 检查mcp.env文件是否存在并包含所需的键
3. 验证凭证值是否正确（特别是JSON格式）
4. 确认manage.sh或make命令实际上在处理secrets
5. 查看日志寻找"正在从凭证源加载"的消息

**解决方案**：
- 重新运行 `make bump` 或 `make init`，确保 secrets 仓库被正确初始化
- 手动检查凭证文件格式
- 验证占位符替换是否正常工作

### 问题：凭证格式导致解析失败
**症形**：配置文件生成后JSON或TOML解析错误

**常见原因**：
- 双引号在JSON值中未正确转义
- 使用了不兼容的引号风格
- 值中包含了换行符或其他不可打印字符

**解决方案**：
- 对于JSON值，强烈建议使用单引号：`KEY='{"json":"value"}'`
- 避免在凭证值中使用换行符
- 使用 `cat -A secrets/mcp.env` 检查隐藏字符
- 参考MCP-CONFIG-GUIDE.md中的TOML转义说明

### 问题：权限导致克隆失败
**症状**：无法从Git仓库克隆secrets

**检查步骤**：
1. 验证网络连接
2. 检查Git凭证或SSH密钥设置
3. 确认仓库URL是否正确且有访问权限
4. 查看错误消息中的具体Git错误

**解决方案**：
- 使用HTTPS URL并确保有访问权限
- 或者配置SSH密钥以访问私有仓库
- 联系仓库管理员确认访问权限
- 作为临时措施，可以手动创建secrets目录和mcp.env文件

### 问题：凭证泄露风险
**症状**：怀疑凭证已暴露或凭证仓库意外公开

**应急措施**：
1. 立即撤销或轮换所有可能暴露的凭证
2. 检查Git历史以确认凭证是否曾提交到公共仓库
3. 检查日志文件以确认凭证是否被意外输出
4. 通知所有可能受影响的服务提供商
5. 审计最近的访问和使用模式

**预防措施**：
- 使用GitHub的secret scanning或类似工具
- 在CI/CD pipeline中添加凭证泄露检查
- 教育团队成员关于凭证安全的最佳实践
- 考虑使用专门的密钥管理服务（如HashiCorp Vault, AWS Secrets Manager）用于高安全需求场景

---

## 与Agent Manager的集成

### 在配置中的使用
在 `config/mcp.json` 中，凭证通过占位符引用：
```json
{
  "servers": {
    "grok-search": {
      "command": "uvx",
      "args": ["--from", "git+https://github.com/GuDaStudio/GrokSearch@<固定commit>", "grok-search"],
      "env": {
        "GROK_API_KEY": "${GROK_API_KEY}"
      }
    }
  }
}
```

Agent Manager 会：
1. 先读取仓库根目录的 `.env`
2. 再读取 `secrets/*.env`（按文件名排序，后者覆盖前者）
3. 若占位符仍未命中，再回退到系统环境变量
4. 替换配置中的 `${GROK_API_KEY}` 占位符
5. 生成 Agent 特定的配置格式
6. 原子写入目标配置文件

额外说明：
- `./manage.sh mcp:apply` 默认会准备 MCP 运行时，并在非 `--offline` 模式下同步本地 secrets Git 仓库
- 如需只消费本地已有凭证，请显式追加 `--offline`
- 真实写入模式下，如果占位符仍未解析，MCP 同步会直接失败，不会写入半成品配置

### 支持的占位符格式
- `${VAR_NAME}`: 标准替换格式
- 环境变量回退：仅在 `.env` 和 `secrets/*.env` 都未命中时，才检查系统环境变量
- 多个占位符：同一文件中可以有多个不同的占位符

### 凭证注入位置
不同Agent的凭�注入位置有所不同：
- JSON 格式的 Claude Code：在 `env` 对象中
- Codex (TOML): 在 `[mcp_servers.*.env]` 节下

---

## 结论

Agent Manager 的凭证管理设计旨在提供安全、便利和可审计的凭证处理方式。通过将凭证与主配置分离存储在独立的Git仓库中，系统实现了：

1. **增强安全**：凭证不会暴露在主仓库或日志中
2. **版本控制**：所有凭证变更都有历史记录
3. **操作简便**：通过标准Git工作流程管理凭证
4. **灵活性**：支持多种凭证类型和格式
5. **企业就绪**：适合需要严格凭证控制的团队和组织

遵循本指南中的最佳实践，可以确保凭证管理的安全性和有效性。
