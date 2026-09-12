import { describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import { DefinitionFormDialog } from "../components/DefinitionFormDialog";

describe("DefinitionFormDialog", () => {
  it("Agent 新建：填写必要命令后提交，无需编辑 JSON 且不伪造未支持字段", () => {
    const onSubmit = vi.fn();
    const onClose = vi.fn();

    render(
      <DefinitionFormDialog
        existingNames={["claude-code", "codex"]}
        onSubmit={onSubmit}
        onClose={onClose}
      />
    );

    expect(screen.getByRole("heading", { name: "添加助手" })).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("唯一名称"), { target: { value: "my-test-agent" } });
    fireEvent.change(screen.getByLabelText("显示名称（可选）"), { target: { value: "My Test Agent" } });
    fireEvent.change(screen.getByLabelText("命令名或可执行文件路径"), { target: { value: "my-agent-cmd" } });
    fireEvent.change(screen.getByLabelText("安装命令（新增时必填）"), { target: { value: "npm install -g my-agent" } });
    fireEvent.change(screen.getByLabelText("升级命令（新增时必填）"), { target: { value: "npm install -g my-agent" } });

    expect(screen.getByText("未填写卸载命令（不支持自动卸载）。")).toBeInTheDocument();
    expect(screen.getByText(/版本检查：my-agent-cmd --version/)).toBeInTheDocument();
    expect(screen.getByText("执行环境：系统默认 Shell")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "保存定义" }));

    expect(onSubmit).toHaveBeenCalledTimes(1);
    const [submittedName, submittedDef] = onSubmit.mock.calls[0];
    expect(submittedName).toBe("my-test-agent");
    expect(submittedDef.display_name).toBe("My Test Agent");
    expect(submittedDef.bin).toBe("my-agent-cmd");
    expect(submittedDef.preferred_provider).toBe("custom");
    expect(submittedDef.providers).toEqual({
      custom: {
        managed: true,
        install: "npm install -g my-agent",
        upgrade: "npm install -g my-agent",
      },
    });

    expect((submittedDef.providers as any).custom.uninstall).toBeUndefined();
    expect(submittedDef.shell).toBeUndefined();
    expect(submittedDef.version_check).toBeUndefined();
  });

  it("无修改保存深相等：未改动字段不具体化 fallback 或改变原始结构", () => {
    const onSubmit = vi.fn();
    const onClose = vi.fn();

    const initial = {
      name: "claude-code",
      definition: {
        display_name: "Claude Code",
        bin: "claude",
        preferred_provider: "native",
        require_managed_provider: true,
        providers: {
          native: {
            managed: true,
            install: {
              posix: "curl -fsSL https://claude.ai/install.sh | bash",
              windows: "curl -fsSL https://claude.ai/install.cmd -o install.cmd",
            },
            upgrade: "claude update",
            uninstall: {
              posix: "rm -f ~/.local/bin/claude",
            },
          },
        },
      },
    };

    render(
      <DefinitionFormDialog
        initial={initial}
        existingNames={["claude-code"]}
        onSubmit={onSubmit}
        onClose={onClose}
      />
    );

    // 直接点击保存，不修改任何字段
    fireEvent.click(screen.getByRole("button", { name: "保存定义" }));

    expect(onSubmit).toHaveBeenCalledTimes(1);
    const [submittedName, submittedDef] = onSubmit.mock.calls[0];
    expect(submittedName).toBe("claude-code");
    // 关键断言：无修改保存必须与原定义深相等，绝不将 posix 额外展开或变更结构
    expect(submittedDef).toEqual(initial.definition);
  });

  it("草稿持久化：渠道切换保留未保存的在途编辑，提交时两渠道改动均保留", () => {
    const onSubmit = vi.fn();
    const onClose = vi.fn();

    const initial = {
      name: "codex",
      definition: {
        display_name: "Codex",
        bin: "codex",
        preferred_provider: "npm",
        providers: {
          native: {
            managed: true,
            install: "curl codex.sh | sh",
            upgrade: "codex update",
          },
          npm: {
            managed: true,
            install: "npm install -g @openai/codex",
            upgrade: "npm install -g @openai/codex",
          },
        },
      },
    };

    render(
      <DefinitionFormDialog
        initial={initial}
        existingNames={["codex"]}
        onSubmit={onSubmit}
        onClose={onClose}
      />
    );

    // 1. 编辑默认渠道 npm 的升级命令
    const upgradeInput = screen.getByLabelText("升级命令（新增时必填）");
    fireEvent.change(upgradeInput, { target: { value: "npm install -g @openai/codex@latest" } });

    // 2. 切换到 native 渠道
    const channelSelect = screen.getByLabelText("管理渠道");
    fireEvent.change(channelSelect, { target: { value: "native" } });
    expect(upgradeInput).toHaveValue("codex update");

    // 3. 编辑 native 渠道的升级命令
    fireEvent.change(upgradeInput, { target: { value: "codex update --beta" } });

    // 4. 切换回 npm 渠道，验证刚才在途编辑的命令仍在
    fireEvent.change(channelSelect, { target: { value: "npm" } });
    expect(upgradeInput).toHaveValue("npm install -g @openai/codex@latest");

    // 5. 提交并验证两边的改动均写入最终定义
    fireEvent.click(screen.getByRole("button", { name: "保存定义" }));

    expect(onSubmit).toHaveBeenCalledTimes(1);
    const [, submittedDef] = onSubmit.mock.calls[0];
    const providers = (submittedDef.providers as any);
    expect(providers.npm.upgrade).toBe("npm install -g @openai/codex@latest");
    expect(providers.native.upgrade).toBe("codex update --beta");
  });

  it("高级设置校验：拒绝 _delete 与 _replace，且 preferred_provider 必须指向存在渠道", () => {
    const onSubmit = vi.fn();
    const onClose = vi.fn();

    render(
      <DefinitionFormDialog
        existingNames={["test-agent"]}
        onSubmit={onSubmit}
        onClose={onClose}
      />
    );

    fireEvent.change(screen.getByLabelText("唯一名称"), { target: { value: "test-agent-new" } });
    fireEvent.change(screen.getByLabelText("命令名或可执行文件路径"), { target: { value: "test-bin" } });
    fireEvent.change(screen.getByLabelText("安装命令（新增时必填）"), { target: { value: "install.sh" } });
    fireEvent.change(screen.getByLabelText("升级命令（新增时必填）"), { target: { value: "upgrade.sh" } });

    // 1. preferred_provider 指向不存在的渠道
    fireEvent.change(screen.getByLabelText("默认安装方式"), {
      target: { value: "non_existent_provider" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存定义" }));
    expect(screen.getByRole("alert")).toHaveTextContent("必须指向已存在的渠道");
    expect(onSubmit).not.toHaveBeenCalled();

    // 恢复合法 preferred_provider
    fireEvent.change(screen.getByLabelText("默认安装方式"), {
      target: { value: "custom" },
    });

    // 2. 高级 JSON 尝试注入 _delete 破坏定义
    fireEvent.change(screen.getByLabelText("其他设置（JSON）"), {
      target: { value: '{"_delete": true}' },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存定义" }));
    expect(screen.getByRole("alert")).toHaveTextContent("不能包含保留键 _delete 或 _replace");
    expect(onSubmit).not.toHaveBeenCalled();

    // 3. 高级 JSON 尝试注入 _replace
    fireEvent.change(screen.getByLabelText("其他设置（JSON）"), {
      target: { value: '{"_replace": true}' },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存定义" }));
    expect(screen.getByRole("alert")).toHaveTextContent("不能包含保留键 _delete 或 _replace");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("取消操作：点击取消按钮或按 Escape 键均调用 onClose", () => {
    const onClose = vi.fn();
    render(
      <DefinitionFormDialog
        existingNames={[]}
        onSubmit={vi.fn()}
        onClose={onClose}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("编辑已有定义：当前平台无安装/升级命令也可保存（不强迫补不支持的命令）", () => {
    const onSubmit = vi.fn();

    // 已有定义只有 posix 命令；当前作用域切到 Windows 后 install/upgrade 提取为空
    const initial = {
      name: "codex",
      definition: {
        display_name: "Codex",
        bin: "codex",
        preferred_provider: "native",
        providers: {
          native: {
            managed: true,
            install: { posix: "curl -fsSL https://chatgpt.com/codex/install.sh | sh" },
            upgrade: { posix: "curl -fsSL https://chatgpt.com/codex/install.sh | sh" },
          },
        },
      },
    };

    render(
      <DefinitionFormDialog
        initial={initial}
        existingNames={["codex"]}
        onSubmit={onSubmit}
        onClose={vi.fn()}
      />
    );

    fireEvent.change(screen.getByLabelText("适用系统"), { target: { value: "windows" } });
    expect(screen.getByLabelText("安装命令（新增时必填）")).toHaveValue("");

    fireEvent.click(screen.getByRole("button", { name: "保存定义" }));

    // 无修改保存必须通过校验且与原定义深相等
    expect(screen.queryByRole("alert")).toBeNull();
    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit.mock.calls[0][1]).toEqual(initial.definition);
  });

  it("编辑 macOS 命令后切到 Linux 保存：不具体化未编辑的 Linux fallback", () => {
    const onSubmit = vi.fn();

    const initial = {
      name: "claude-code",
      definition: {
        display_name: "Claude Code",
        bin: "claude",
        preferred_provider: "native",
        providers: {
          native: {
            managed: true,
            install: { posix: "curl old.sh | bash" },
            upgrade: "claude update",
          },
        },
      },
    };

    render(
      <DefinitionFormDialog
        initial={initial}
        existingNames={["claude-code"]}
        onSubmit={onSubmit}
        onClose={vi.fn()}
      />
    );

    // 编辑 macOS（darwin）安装命令
    fireEvent.change(screen.getByLabelText("适用系统"), { target: { value: "darwin" } });
    fireEvent.change(screen.getByLabelText("安装命令（新增时必填）"), { target: { value: "curl new-mac.sh | bash" } });

    // 切到 Linux（不做任何编辑）直接保存
    fireEvent.change(screen.getByLabelText("适用系统"), { target: { value: "linux" } });
    fireEvent.click(screen.getByRole("button", { name: "保存定义" }));

    expect(onSubmit).toHaveBeenCalledTimes(1);
    const install = (onSubmit.mock.calls[0][1].providers as any).native.install;
    // darwin 被编辑，posix fallback 保持原样，不得具体化出未编辑的 linux 键
    expect(install).toEqual({ posix: "curl old.sh | bash", darwin: "curl new-mac.sh | bash" });
    expect(install.linux).toBeUndefined();
  });

  it("焦点环：唯一名称 Shift+Tab 不逃到背景，循环到对话框末尾按钮", () => {
    render(
      <div>
        <button type="button">背景按钮</button>
        <DefinitionFormDialog existingNames={[]} onSubmit={vi.fn()} onClose={vi.fn()} />
      </div>
    );

    const nameInput = screen.getByLabelText("唯一名称");
    nameInput.focus();
    expect(document.activeElement).toBe(nameInput);

    const submitButton = screen.getByRole("button", { name: "保存定义" });
    fireEvent.keyDown(nameInput, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(submitButton);
    expect(document.activeElement).not.toBe(screen.getByText("背景按钮"));

    // 末尾再 Tab 循环回第一个可聚焦元素
    fireEvent.keyDown(submitButton, { key: "Tab" });
    expect(document.activeElement).toBe(nameInput);
  });

  it("单渠道时管理渠道为只读名称，不可改名制造空 provider", () => {
    render(
      <DefinitionFormDialog existingNames={[]} onSubmit={vi.fn()} onClose={vi.fn()} />
    );

    // 只读文本展示，不提供可编辑 input（改名会丢草稿命令并假通过验证）
    expect(screen.getByText("custom")).toBeInTheDocument();
    expect(screen.queryByLabelText("管理渠道")).toBeNull();
  });

  it("关闭后焦点恢复到触发按钮", () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <div>
          <button type="button" onClick={() => setOpen(true)}>添加助手</button>
          {open && (
            <DefinitionFormDialog existingNames={[]} onSubmit={vi.fn()} onClose={() => setOpen(false)} />
          )}
        </div>
      );
    }

    render(<Harness />);
    const trigger = screen.getByRole("button", { name: "添加助手" });
    trigger.focus();
    fireEvent.click(trigger);
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });
});
