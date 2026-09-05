/** 占位页：Phase 1 后续切片实现的页面先渲染此组件。 */
export function PlaceholderPage({ title }: { title: string }) {
  return (
    <div className="rounded-lg border border-dashed border-slate-300 bg-white p-12 text-center">
      <h2 className="text-lg font-semibold text-slate-700">{title}</h2>
      <p className="mt-2 text-sm text-slate-400">本页将在 Phase 1 后续切片实现。</p>
    </div>
  );
}
