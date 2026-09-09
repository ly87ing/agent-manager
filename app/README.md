# app/ — Agent Manager 桌面客户端

目标为 Tauri 2 + React 界面直接调用 Rust 库。产品语义见[重设计方案](../docs/GUI-REDESIGN.md)，实施顺序见[任务书](../docs/goal-rust-gui.md)。

**当前处于接线重构阶段。** 现有 Tauri 桥和打包配置仍有旧 Python 路径，不能把旧独立资源包当作新版 Rust 交付物。

## 界面与调用要求

- 日常导航为助手、技能、MCP、规则与 Hooks、操作记录、设置，默认打开助手。
- 保存定义只保存本地资源；安装和应用是独立动作。预览只读，取消零写入。
- Skills 集合、MCP 服务库和逐助手分配分别展示操作范围；按钮使用添加、更新、启用、禁用、从此助手删除等明确文案。
- `engine_invoke` 经工作线程直接调用 `engine-rs`；保留并校验 envelope，未实现命令返回明确错误。
- GUI 不直接写 SQLite 或助手配置，不执行旧 `manage.sh`，不复制 Python 引擎到数据目录。
- 操作记录显示实际结果；默认演练偏好只决定预览复选框初值，不能强制每次执行变成演练。
- 纯浏览器 mock 仅用于页面验证，不代表 Rust 或真实助手接入通过。

## 开发入口

在本目录运行：

```bash
pnpm install
pnpm dev
pnpm typecheck
pnpm test:unit run
pnpm build
```

`pnpm dev` 为前端开发服务器；可用 `?mock=1` 查看测试数据。真实引擎调用需要 Tauri。Rust 检查从仓库根运行：

```bash
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
```

`pnpm tauri dev` 和 `pnpm tauri build` 是现有桌面入口；原生接线完成前可能仍走旧链路，不作为新版通过证据。打包必须先去除 `beforeBuildCommand` 的 Python 资源构建和 `resources/engine` 旧运行时依赖。

## 交付验收

从最终提交重建包，在无 Python、无旧脚本、无源码访问的隔离环境中启动。通过真实 UI 保存和应用资源，核对 SQLite 与两个测试助手目标文件，再重启读回；取消、演练、失败恢复、外部修改、共享目标均需反例。

检查正常窗口、小窗口及 200% 缩放下的文字、滚动、焦点、弹窗关闭和错误反馈。包内只含程序、界面和通用只读资源；用户命令、状态及 Skill 版本保存在应用数据目录。

平台和签名情况分别记录在[平台状态](../docs/GUI-PLATFORM-STATUS.md)。未配置真实更新材料时明确显示未配置；不自动下载、安装或重启。推送分支不代表授权发布安装包。
