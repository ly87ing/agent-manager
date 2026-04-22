# CC-Bridge Runtime 管理

本文档说明如何用当前仓库以“外围托管”的方式管理 `cc-bridge`：

- `agent-manager` 负责 secrets、版本发现、运行目录渲染、安装 / 启动 / 停止 / 升级入口和回滚入口
- `Docker` 负责容器生命周期
- 真实敏感信息只允许放在外部私有 `secrets/` 仓库

## 运行边界

- 本方案当前只支持单机 `SQLite`
- `DATABASE_DRIVER` 必须是 `sqlite`
- 数据持久化固定使用宿主机 bind mount，不使用 Docker named volume
- 运行目录默认是 `~/services/cc-bridge`，可用 `CC_BRIDGE_RUNTIME_DIR` 覆盖

生成后的宿主机运行目录形态：

```text
~/services/cc-bridge/
├── compose.yaml
├── .env.rendered
├── data/
├── backups/
└── state/
```

其中：

- `data/`：挂载到容器 `/app/data`
- `backups/`：每次升级前自动生成 `tar.gz` 备份
- `state/`：记录当前 tag、上一个 tag、最近一次成功升级时间

## Secrets 文件

外部私有 `secrets/` 仓库至少需要以下文件：

### `secrets/cc-bridge.env`

```env
CC_BRIDGE_RUNTIME_DIR=/Users/ly/services/cc-bridge
SERVER_HOST=0.0.0.0
SERVER_PORT=5674
DATABASE_DRIVER=sqlite
LOG_LEVEL=info
ADMIN_PASSWORD=YOUR_STRONG_PASSWORD
```

可选键：

- `CC_BRIDGE_IMAGE_REPO`

默认值：

- `CC_BRIDGE_IMAGE_REPO=ghcr.io/mamoworks/cc-bridge`

### `secrets/cc-bridge.accounts.json`

```json
[
  {
    "name": "YOUR_ACCOUNT_NAME",
    "email": "YOUR_EMAIL",
    "auth_type": "setup_token",
    "setup_token": "YOUR_SETUP_TOKEN",
    "account_uuid": "YOUR_ACCOUNT_UUID",
    "organization_uuid": "YOUR_ORG_UUID",
    "subscription_type": "pro"
  }
]
```

### `secrets/cc-bridge.api-tokens.json`

```json
[
  {
    "name": "default-client",
    "allowed_accounts": "",
    "blocked_accounts": ""
  }
]
```

## 自动更新策略

- 安装、启动和更新命令都不会使用漂移的 `:latest`
- 每次执行 `make cc-bridge-install`、`make cc-bridge-start`、`make cc-bridge-deploy` 或 `make cc-bridge-update` 时，脚本都会读取：

```text
https://api.github.com/repos/MamoWorks/cc-bridge/releases/latest
```

- 解析 `tag_name`
- 以显式 tag 渲染本机 `compose.yaml`
- 执行 `docker compose down`
- 备份宿主机 `data/`
- 再执行 `docker compose pull && docker compose up -d`

其中：

- `make cc-bridge-start` 启动前会先检查最新稳定版本
- 如果当前未安装，就按最新稳定版本完成安装并启动
- 如果当前已安装但发现新版本，就先升级再启动
- 如果当前已经是最新稳定版本，只执行 `docker compose up -d`

这样做的结果是：

- 你不需要自己查新版本
- 运行时仍然是固定 tag，方便审计和回滚

## 命令

### 首次安装并启动

```bash
make cc-bridge-install
```

### 启动服务

```bash
make cc-bridge-start
```

### 停止服务

```bash
make cc-bridge-stop
```

### 兼容旧入口：首次部署

```bash
make cc-bridge-deploy
```

### 升级到最新稳定版本

```bash
make cc-bridge-update
```

### 导入账号池和 API token

```bash
make cc-bridge-seed
```

### 查看当前状态

```bash
make cc-bridge-status
```

状态输出会包含：

- 当前 tag、上一个 tag、最新稳定 tag
- 是否存在可升级版本
- 容器运行状态和容器主进程 PID
- 运行目录、访问 URL、登录 URL、管理 URL 与端口

### 回滚到上一个版本

```bash
make cc-bridge-rollback
```

## 失败模式与回滚

- 如果 `releases/latest` 查询失败，安装 / 启动 / 部署 / 更新都会直接失败，不会猜测版本
- 如果 `cc-bridge.env` 缺失或 `ADMIN_PASSWORD` 为空，脚本会失败关闭
- 如果 `DATABASE_DRIVER` 不是 `sqlite`，脚本会拒绝继续，避免半支持状态
- 如果升级后 healthcheck 失败，更新路径会自动尝试回滚到更新前记录的当前 tag
- 数据目录备份保留在宿主机 `backups/` 中；如需回滚数据，请手工从备份恢复

## 当前实现说明

- 本方案不会修改 `manage.sh` 主路由
- 入口是 `Makefile` 的 `cc-bridge-*` 目标和 `scripts/cc-bridge/manage.py`
- 这保持了现有 `agent-manager` 的 `agents/tools/mcp/skills/rules` 主抽象不变，只增加一个受控的外部服务管理面
