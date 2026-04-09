# Remote Codex iPhone Access Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付一个单文件响应式 HTML 页面，完整说明 iPhone 通过 Tailscale 和 SSH 远程连接 Mac 上 Codex CLI 的架构与操作流程。

**Architecture:** 页面放在 `docs/` 下，采用单页文档结构，内联样式和少量脚本实现导航、滚动高亮和响应式布局。内容以“架构解释 + 操作清单 + 全链路消息流”为主，确保用户可以直接照着配置。

**Tech Stack:** HTML, CSS, vanilla JavaScript

---

### Task 1: 落盘设计文档

**Files:**
- Create: `docs/superpowers/specs/2026-04-09-remote-codex-ios-mac-design.md`

- [ ] **Step 1: 写出方案设计**

补全目标、链路、信息结构和视觉方向。

- [ ] **Step 2: 自查信息边界**

确认方案聚焦在 `Tailscale + SSH + tmux + Codex CLI`，不混入 `WorkBuddy` 或消息派单方案。

### Task 2: 实现单文件 HTML 页面

**Files:**
- Create: `docs/remote-codex-ios-mac.html`

- [ ] **Step 1: 搭建页面骨架**

创建头部、导航、主内容区和章节锚点。

- [ ] **Step 2: 实现视觉层**

加入响应式布局、色彩变量、Hero 链路图和轻量动效。

- [ ] **Step 3: 写入说明内容**

补齐架构、原理、消息流、Mac/iPhone 操作和 FAQ。

- [ ] **Step 4: 实现轻量交互**

用原生 JavaScript 实现当前章节高亮与移动端导航体验。

### Task 3: 验证页面可用性

**Files:**
- Verify: `docs/remote-codex-ios-mac.html`

- [ ] **Step 1: 打开本地 HTML**

用浏览器加载仓库内的 `docs/remote-codex-ios-mac.html`。

- [ ] **Step 2: 检查桌面端布局**

确认导航、章节跳转、链路图和代码块均正常显示。

- [ ] **Step 3: 检查移动端布局**

确认导航、字号、段落宽度和代码区域可阅读。

- [ ] **Step 4: 汇报验证结果**

基于实际查看结果说明页面是否可用，以及是否存在剩余限制。
