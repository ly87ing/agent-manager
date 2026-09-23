import { act, waitFor } from "@testing-library/react";
import { EditorView } from "@codemirror/view";
import { screen } from "./render-with-engine";

/** 等懒加载的 CodeMirror 挂好，取它的 EditorView。 */
export async function editorView(testId: string): Promise<EditorView> {
  const host = await screen.findByTestId(testId);
  let view: EditorView | null = null;
  await waitFor(() => {
    const dom = host.querySelector(".cm-editor");
    view = dom ? EditorView.findFromDOM(dom as HTMLElement) : null;
    if (!view) throw new Error("编辑器还没挂好");
  });
  return view!;
}

export function editorText(view: EditorView): string {
  return view.state.doc.toString();
}

/** 像用户编辑一样整篇替换内容（走编辑器事务，触发 onChange）。 */
export function setEditorText(view: EditorView, text: string): void {
  act(() => {
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
  });
}
