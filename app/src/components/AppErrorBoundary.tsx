import { Component, type ReactNode } from "react";
import { AlertOctagon, RotateCcw } from "lucide-react";

/**
 * 顶层渲染错误边界：任何页面组件抛错时兜住整棵树，给出中文恢复界面，
 * 「重试」重置错误状态重新渲染；反复失败时提示重启应用。
 */
export class AppErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: { componentStack?: string | null }) {
    console.error("界面渲染异常", error, info.componentStack);
  }

  private retry = () => {
    this.setState({ error: null });
  };

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="flex min-h-screen items-center justify-center bg-background p-6">
        <div role="alert" className="w-full max-w-lg rounded-xl border border-destructive/30 bg-card p-6 shadow-sm">
          <div className="flex items-center gap-2">
            <AlertOctagon className="h-5 w-5 text-destructive" aria-hidden />
            <h2 className="text-base font-semibold text-destructive">界面出现异常</h2>
          </div>
          <p className="mt-3 text-sm text-muted-foreground">
            界面渲染时发生未预期的错误，已停止在当前状态。你可以点击「重试」重新渲染；如果重试后仍然出现异常，请重启应用。
          </p>
          <pre className="mt-3 max-h-40 overflow-auto rounded-md bg-destructive/10 p-3 font-mono text-xs text-destructive">
            {error.message}
          </pre>
          <div className="mt-4 flex justify-end">
            <button
              type="button"
              onClick={this.retry}
              className="inline-flex items-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
            >
              <RotateCcw className="h-4 w-4" aria-hidden />
              重试
            </button>
          </div>
        </div>
      </div>
    );
  }
}
