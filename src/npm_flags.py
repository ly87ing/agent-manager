#!/usr/bin/env python3
"""受管全局 npm 安装的统一加固参数。

sync_agents 与 sync_tools 曾各持一份实现。两份漂移出的差异——tools 侧漏了
--registry——让版本锁按官方 registry 写入、安装却落到本机 registry,本机若配的是
滞后镜像,锁到的新版本在那里还不存在,npm install 直接 ETARGET。两边共用这一份,
避免再次分叉。
"""

OFFICIAL_NPM_REGISTRY = "https://registry.npmjs.org"

GLOBAL_INSTALL_PREFIX = "npm install -g"

_VALUED_FLAGS = (
    ("--registry", OFFICIAL_NPM_REGISTRY),
    ("--fetch-retries", "2"),
    ("--fetch-retry-factor", "2"),
    ("--fetch-retry-mintimeout", "10000"),
    ("--fetch-retry-maxtimeout", "60000"),
    ("--fetch-timeout", "120000"),
)

_BARE_FLAGS = ("--no-audit", "--no-fund")


def inject_npm_flags(cmd):
    """给全局 npm 安装命令补齐 registry / 重试 / 超时加固,已显式指定的参数不覆盖。

    非字符串(如列表形式的命令)与非全局安装命令原样返回。
    """
    if not isinstance(cmd, str):
        return cmd

    stripped = cmd.strip()
    if not stripped.startswith(GLOBAL_INSTALL_PREFIX):
        return stripped

    hardened = stripped
    for flag, value in _VALUED_FLAGS:
        if f"{flag} " in hardened or f"{flag}=" in hardened:
            continue
        hardened = f"{hardened} {flag} {value}"
    for flag in _BARE_FLAGS:
        if flag not in hardened:
            hardened = f"{hardened} {flag}"
    return hardened
