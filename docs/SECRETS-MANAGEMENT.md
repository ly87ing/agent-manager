# 密钥配置

## 手动配置

安装和使用不需要作者的 secrets 仓库，也不需要 GitHub 账号。

1. 打开「设置 → 密钥」，点「新建一个」，或填入自己的外部 `.env` 路径后点「验证并使用」。默认新建到 `~/.config/agent-manager/secrets.env`。
2. 添加 API 账号时填写自己的 API Key；MCP 所需变量可在密钥页填写。页面显示变量名和是否已配置，不回显取值。
3. 确认文件状态为「已连接」。文件不可用时先修复路径、格式或权限。切换文件不会迁移旧值，也不会更新已生效的助手配置。

## 使用自己的私有仓库

仓库地址、文件名和托管平台由你选择。推荐在项目外存放如下结构；现有 `mcp.env`、`secrets.env` 均可直接使用，无需迁移。

```text
YOUR_PRIVATE_SECRETS/
  README.md       # 用途说明，不列出真实取值
  mcp.env         # 应用选择的密钥文件
  mcp.env.example # 仅占位符，可用于初始化
```

先在托管平台创建**私有仓库**并配置本机 Git 访问权限，再克隆到项目外。安装包用户可用常规 `git clone`；源码用户也可使用：

```sh
mkdir -p "$HOME/.config/agent-manager"
bash scripts/init-secrets.sh YOUR_PRIVATE_REPO_URL "$HOME/.config/agent-manager/secrets"
```

在私有仓库中创建或编辑 `mcp.env`，macOS / Linux 执行 `chmod 600 mcp.env`，然后在应用中选择该文件。Windows 请用文件安全属性限制为自己的账户可访问；当前引擎不自动检查 Windows ACL。

Git 同步由用户主动操作。应用与初始化脚本都不会自动拉取、提交或推送；目标目录已存在时脚本停止。拉取冲突先在 Git 中解决，再回到应用验证。停止使用只取消文件引用，不删除仓库或凭据。

## .env 格式

一行一个变量，变量名使用字母、数字和下划线，首字符不能是数字；同名变量不能重复。支持空行、整行 `#` 注释和可选的 `export ` 前缀。双引号值按 JSON 字符串解析，单引号值按字面量解析；不执行 Shell 命令，也不展开变量。行尾注释会被当作值的一部分。

```dotenv
YOUR_PROVIDER_API_KEY="YOUR_API_KEY"
YOUR_MCP_TOKEN="YOUR_TOKEN"
```

把变量名与账号或 MCP 中引用的名称保持一致，例如 `${YOUR_MCP_TOKEN}`。示例值必须换成自己的真实值。一个应用实例当前只选择一个文件；终端专用凭据可留在其他文件，不需要合并。

## 发布者密钥

Tauri 更新签名私钥、密码及商业签名证书只属于维护者，通过 GitHub Actions Secrets 注入构建。普通用户不配置这些内容；安装包只含验签公钥。维护者可在自己的私有仓库用独立目录备份发布密钥，不能把这个目录选作用户运行时的密钥来源。

## 当前规则

生效配置以各助手真实文件为准；应用数据库只保存偏好、归属记录和未生效备选。

- 真实凭据只通过用户明确选择的外部私有 `.env` 文件或进程环境提供。Agent Manager 不把明文写入根仓库、SQLite、日志、配置备份或测试产物。
- SQLite definition/binding 只保存 `${VAR}` 占位符，`app_settings` 只保存外部文件路径。status、诊断和操作日志只报告变量名、来源与布尔状态，不保存或展示真实值。
- 选择已有文件时，它必须是普通文件、Unix 权限不向组或其他用户开放、内容为合法 `KEY=VALUE`，且不能位于应用数据目录。显式新建使用 `600` 权限，不覆盖已有文件。应用不会自动克隆或猜测 secrets 仓库。
- apply 生成的目标客户端配置是否包含解析后的值取决于客户端格式，安全性依赖目标文件权限和客户端安全边界。
- 运行日志和错误响应不得包含 token、私钥、完整连接串或其它敏感值。
- 查询与 status 只读零写；公共 `*:plan` 与 `--dry-run` 入口已退役，调用会被拒绝。

## MCP 与 Agent 配置

MCP 以各助手配置文件里的实际条目为准，`native-manager.db` 只保存「已保存未生效」的 MCP 项（Claude 侧停用的快照、还没写进任何助手的定义），不另存当前配置；见 [MCP 配置指南](MCP-CONFIG-GUIDE.md)。写入条目时新设的 env `${VAR}` 按助手解析取值、headers 以客户端原生引用形式写入；新写的 env 字面量命中疑似凭据检测时拒绝。外部文件中的取值优先，进程环境只作只读回退；没有可用外部文件时，界面填写与需要转存密钥的 Claude 停用 fail-closed。

`secrets:source` 只保存路径；`secrets:set` 只更新指定变量；停用 Claude MCP 条目时，疑似凭据的明文取值由 MCP 域转存到该文件（只补缺失值，同名不同值拒绝覆盖），快照里只留 `${VAR}`。旧版 `secret_values` 表只用于一次性迁移：页面显示数量但不读取回显，用户输入确认词后才执行；引擎先把全部旧值无覆盖写入外部文件，成功后再删除旧表，任一步失败都保留旧表。配置导出永不包含取值或外部文件路径，导入遇到旧版非空 `secrets` 段会拒绝。

Claude Code 与 Codex 的内置默认安装渠道是官方 `native` 命令。brew 只作为实际检测来源展示，不是默认渠道；凭据本身仍由对应工具和用户环境管理。

## 验证与报告

使用隔离的 `AGENT_MANAGER_DATA_DIR` 做本地验证，不覆盖 HOME。提交前运行：

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
cd app && pnpm test:unit run && pnpm typecheck && pnpm build
```

若错误输出疑似包含敏感信息，应立即停止传播，清理临时产物，并改用脱敏后的错误类别和路径报告。
