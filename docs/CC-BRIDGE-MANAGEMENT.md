# CC-BRIDGE 管理说明（受控文档）

该文档仅保留最小接入信息；完整实现细节与敏感参数已从公开内容中移除，仅对授权维护路径开放。

## 入口命令

- `make cc-bridge-install`: 首次安装并启动
- `make cc-bridge-start`: 启动服务
- `make cc-bridge-stop`: 停止服务
- `make cc-bridge-deploy`: 兼容旧部署入口
- `make cc-bridge-update`: 更新到最新稳定版
- `make cc-bridge-seed`: 导入账号与 token（内部凭据场景）
- `make cc-bridge-status`: 查看当前运行与版本状态
- `make cc-bridge-rollback`: 回滚到上一个可用版本

## 说明

- 当前文档不再公开展示运行目录、镜像细节、版本发现流程与 secrets 字段，避免在通用文档中暴露额外环境信息。
- 需要执行或排障时，请使用仓库内授权流程获取完整运行手册。
