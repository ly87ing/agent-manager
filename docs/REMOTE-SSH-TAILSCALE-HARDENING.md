# Mac Remote SSH + Tailscale 加固方案

> 目标：让 iPhone 通过 Tailscale 私网 SSH 到这台 Mac，继续使用本机 `Codex CLI`，同时满足以下约束：
>
> - 只允许指定来源设备访问 `22` 端口
> - 只允许 SSH key 登录
> - 不允许账号密码方式进行远程 SSH 登录
> - 本机坐在电脑前登录、解锁、`sudo` 仍然可以正常使用账号密码

---

## 适用前提

本方案默认前提是：

- 使用 **普通 SSH over Tailscale**
- 使用 macOS 自带的 `Remote Login`
- **不启用 `Tailscale SSH`**

原因：

- 这条链路最贴近原生终端工作流：`iPhone -> Tailscale -> SSH -> tmux -> Codex CLI`
- `sshd_config`、`authorized_keys` 和本机 SSH 行为完全由你自己控制
- 如果后续切换成 `Tailscale SSH`，来自 tailnet 的 `22` 端口会由 Tailscale 接管，控制面会转到 Tailscale policy，而不是本机 `sshd_config`

---

## 敏感信息边界

本文档中的用户名、设备名、域名、Tailscale IP 和 SSH 示例值全部使用占位符。

- 真实的 `macOS` 用户名
- 真实的 Tailscale 设备名和完整域名
- 真实的 Tailscale IPv4 地址
- 真实的 SSH Host 配置和私钥标识

这些值都应存放在 secret 库或等价的私密配置仓库中，不应提交到当前仓库的代码、文档或示例里。

---

## 当前机器状态

基于当前这台 Mac 的本地检查，可确认：

- `com.openssh.sshd` 已启用
- 本机 `127.0.0.1:22` 可连接
- 当前磁盘上的 [`/etc/ssh/sshd_config`](/etc/ssh/sshd_config) 已经启用了“只允许 key、禁密码”的基础限制

当前主配置中的关键项为：

```sshconfig
PermitRootLogin no
PubkeyAuthentication yes
AuthenticationMethods publickey
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitEmptyPasswords no
AllowUsers YOUR_MAC_USER
```

这说明：

- 远程 SSH 已经是 **key-only**
- 远程 SSH 已经 **禁用密码登录**
- 当前剩余缺口主要是：
  - 还没有把来源设备限制到指定 iPhone
  - 还没有在 Tailscale policy 层明确收紧 `22` 端口入口

---

## 最终推荐架构

完整方案建议分成三层控制，不要把所有安全策略都塞进 [`/etc/ssh/sshd_config`](/etc/ssh/sshd_config)。

### 第一层：Remote Login

在 macOS 的 `General -> Sharing -> Remote Login` 中：

- 打开 `Remote Login`
- 只允许自己的 mac 账号登录
- 不要使用 `All users`

这一层解决的是：

- 哪些本机账号有资格进行远程登录

### 第二层：Tailscale policy

在 tailnet 的 `Access controls` 中：

- 只允许 `你的 iPhone -> 这台 Mac:22`
- 不允许 tailnet 中其他设备访问这台 Mac 的 SSH 端口

这一层解决的是：

- 哪些来源设备能碰到 `22` 端口

### 第三层：OpenSSH (`sshd_config`)

在本机 SSH 配置中：

- 只允许用户 `YOUR_MAC_USER`
- 只允许指定 iPhone 的 Tailscale IP 作为来源
- 只允许 `publickey`
- 禁止密码登录
- 禁止 `root` 远程登录

这一层解决的是：

- 即使有人进入了 tailnet 访问路径，也必须同时满足用户名、来源地址和 key 三个条件

---

## 你真正想达到的效果

最终状态应该是：

- iPhone 通过 `SSH key` 可以登录
- iPhone 通过账号密码不能登录
- tailnet 中其他设备无法访问这台 Mac 的 `22`
- 本机坐在电脑前登录系统、解锁、`sudo` 仍然正常使用账号密码

也就是说：

- **远程 SSH 认证方式** 被收紧
- **本机用户密码体系** 不受影响

---

## 实施前先收集 4 个值

在开始修改前，先确认以下信息：

1. 你的 mac 用户名，例如 `YOUR_MAC_USER`
2. 这台 Mac 的 Tailscale 设备名，例如 `YOUR_MAC_DEVICE_NAME`
3. 这台 Mac 的 Tailscale 完整域名，例如 `YOUR_MAC_TAILNET_FQDN`
4. 你的 iPhone 的 Tailscale IPv4 地址，例如 `YOUR_IPHONE_TAILSCALE_IP`

其中最关键的是第 4 项，因为后续会用于来源限制。

---

## 推荐实施顺序

### 步骤 1：确认公钥已经就位

先确认 iPhone 端对应的公钥已经在本机用户家目录的 `~/.ssh/authorized_keys` 中。

推荐权限：

```bash
chmod 700 ~/.ssh
chmod 600 ~/.ssh/authorized_keys
```

如果这里没有准备好，不要继续往下收紧配置。

### 步骤 2：备份当前 SSH 主配置

```bash
sudo cp /etc/ssh/sshd_config /etc/ssh/sshd_config.bak-$(date +%Y%m%d%H%M%S)
```

### 步骤 3：决定配置落点

这台机器目前已经直接在主文件 [`/etc/ssh/sshd_config`](/etc/ssh/sshd_config) 底部写了自定义限制。

有两种做法：

#### 做法 A：最小改动，适合当前机器

继续使用主文件，只把：

```sshconfig
AllowUsers YOUR_MAC_USER
```

改成：

```sshconfig
AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP
```

这里的 `YOUR_IPHONE_TAILSCALE_IP` 换成你那台 iPhone 的真实 Tailscale IP。

#### 做法 B：后续整理，改成 drop-in 文件

本机已经支持 [`/etc/ssh/sshd_config.d`](/etc/ssh/sshd_config.d)，且主配置开头已有：

```sshconfig
Include /etc/ssh/sshd_config.d/*
```

如果后续想把本地自定义与系统默认分离，可以新建：

[`/etc/ssh/sshd_config.d/900-local-hardening.conf`](/etc/ssh/sshd_config.d/900-local-hardening.conf)

写入：

```sshconfig
PermitRootLogin no
PubkeyAuthentication yes
AuthenticationMethods publickey
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitEmptyPasswords no
AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP
```

然后把主文件底部重复的自定义项清理掉，避免多处维护。

当前阶段更建议先走 **做法 A**，因为这台机器已经在主文件里维护这些规则。

---

## `sshd_config` 最终建议

假设：

- 用户名是 `YOUR_MAC_USER`
- iPhone 的 Tailscale IP 是 `YOUR_IPHONE_TAILSCALE_IP`

则最终推荐的 SSH 限制应为：

```sshconfig
PermitRootLogin no
PubkeyAuthentication yes
AuthenticationMethods publickey
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitEmptyPasswords no
AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP
```

### 各项含义

- `PermitRootLogin no`
  - 禁止 root 直接远程登录
- `PubkeyAuthentication yes`
  - 开启公钥认证
- `AuthenticationMethods publickey`
  - 强制只接受公钥认证
- `PasswordAuthentication no`
  - 禁止远程密码登录
- `KbdInteractiveAuthentication no`
  - 禁止 keyboard-interactive 交互式口令认证
- `PermitEmptyPasswords no`
  - 禁止空密码
- `AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP`
  - 只允许用户 `YOUR_MAC_USER` 且来源地址是这台 iPhone

### 不建议的写法

不要为了省事直接放宽成整个 Tailscale IPv4 地址池：

```sshconfig
AllowUsers YOUR_MAC_USER@100.64.0.0/10
```

这技术上可行，但安全边界太宽。既然已经决定做双重限制，就应该写成单个设备的 Tailscale IP。

---

## Tailscale policy 方案

来源设备限制更适合放在 Tailscale policy，而不是只靠 `sshd_config`。

### 推荐目标

只允许：

- `你的 iPhone -> 你的 Mac:22`

不允许：

- tailnet 里其他设备访问这台 Mac 的 `22`

### `grants` 风格示例

如果你的 tailnet policy 已经使用 `grants` 风格，推荐直接用 `hosts` 别名配合 `grants`：

```json
{
  "hosts": {
    "mac_remote": "YOUR_MAC_TAILSCALE_IP",
    "iphone_remote": "YOUR_IPHONE_TAILSCALE_IP"
  },
  "grants": [
    {
      "src": ["iphone_remote"],
      "dst": ["mac_remote"],
      "ip": ["tcp:22"]
    }
  ]
}
```

说明：

- `src` 和 `dst` 都可以使用 `hosts` 中定义的别名
- `ip` 中只开放 `tcp:22`
- 对新的 tailnet policy，优先使用 `grants`

### `acls` 风格示例

如果你的 tailnet 还是 `acls` 风格，可以参考：

```json
{
  "hosts": {
    "mac_remote": "YOUR_MAC_TAILSCALE_IP",
    "iphone_remote": "YOUR_IPHONE_TAILSCALE_IP"
  },
  "acls": [
    {
      "action": "accept",
      "src": ["iphone_remote"],
      "dst": ["mac_remote:22"]
    }
  ]
}
```

说明：

- 不要直接整份覆盖现有 policy
- 如果已有其他业务规则，应把这条规则合并进去
- `hosts` 中的名称只是便于阅读，不是必须使用的名字

---

## 为什么两层都应该做

### 只做 `sshd_config` 的问题

如果只做：

- `AllowUsers YOUR_MAC_USER@100.x.y.z`
- `PasswordAuthentication no`

那么仍然存在一个问题：

- 其他 tailnet 设备依然能碰到你的 `22`
- 它们只是最终登录会失败

这不如直接在 Tailscale 层面把入口封死。

### 只做 Tailscale ACL 的问题

如果只做：

- `iPhone -> Mac:22`

而不收紧 `sshd_config`，那么一旦 SSH key / 用户配置出问题，或者你后面误打开了密码认证，远程登录边界就会退化。

### 两层一起做的好处

- Tailscale 负责“谁能来”
- OpenSSH 负责“来了之后谁能进”

这是最清晰、最稳的边界划分。

---

## 实际执行顺序

严格按照这个顺序操作，避免把自己锁死：

1. 确认 iPhone 的公钥已经写入 `~/.ssh/authorized_keys`
2. 确认当前通过 SSH key 登录本机是可行的
3. 备份 [`/etc/ssh/sshd_config`](/etc/ssh/sshd_config)
4. 将 `AllowUsers YOUR_MAC_USER` 收紧为 `AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP`
5. 校验配置语法
6. 不关闭当前会话，新开一个 SSH 测试会话
7. 验证 key 登录成功
8. 验证密码登录失败
9. 再去 Tailscale `Access controls` 收紧 `22` 端口入口
10. 最后用 iPhone 的 `Termius` 做真实连接验证

---

## 验证命令

### 1. 校验 SSH 配置语法

```bash
sudo /usr/sbin/sshd -t
```

无输出即为通过。

### 2. 验证 key 登录成功

从另一台设备或另一个终端测试：

```bash
ssh YOUR_MAC_USER@YOUR_MAC_TAILNET_FQDN
```

或者：

```bash
ssh YOUR_MAC_USER@YOUR_MAC_TAILSCALE_IP
```

应当成功进入 shell。

### 3. 验证密码登录失败

强制禁用 key，仅尝试密码 / keyboard-interactive：

```bash
ssh \
  -o PubkeyAuthentication=no \
  -o PreferredAuthentications=password,keyboard-interactive \
  YOUR_MAC_USER@YOUR_MAC_TAILNET_FQDN
```

预期结果：

- 不应成功登录
- 应提示认证失败，而不是进入 shell

### 4. 验证非授权来源失败

从不是你 iPhone 的其他 tailnet 设备测试：

- 如果 Tailscale ACL 生效，应在网络层就被拒绝
- 即使 ACL 暂未生效，`AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP` 也应阻止登录成功

---

## 回滚方案

如果改完发现自己被锁在外面：

1. 回到本机桌面或本地终端
2. 恢复备份：

```bash
sudo cp /etc/ssh/sshd_config.bak-YYYYMMDDHHMMSS /etc/ssh/sshd_config
```

3. 再次校验：

```bash
sudo /usr/sbin/sshd -t
```

4. 重新测试 SSH

所以在完成新会话验证前，**不要关闭当前仍然可用的会话**。

---

## 不要混入的方案

以下方案不要和当前方案混用，否则控制面会变复杂：

- `Tailscale SSH`
- 公网暴露 `22` 端口
- 只靠 macOS 图形界面 Firewall 做来源白名单
- 在 `sshd_config` 中放宽整个 `100.64.0.0/10`

---

## 对这台机器的直接建议

结合当前机器实际状态，最小可落地动作只有两件：

1. 保持现有 key-only 配置不变
2. 将：

```sshconfig
AllowUsers YOUR_MAC_USER
```

改为：

```sshconfig
AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP
```

然后再补一条 Tailscale policy，只允许这台 iPhone 访问这台 Mac 的 `22`。

这样既不会改变本机账号密码使用习惯，也能把远程 SSH 真正收紧到“只有你的手机 + 只有你的 key”。
