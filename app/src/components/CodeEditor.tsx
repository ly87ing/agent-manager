import { useEffect, useRef } from "react";
import { basicSetup } from "codemirror";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, placeholder as placeholderText } from "@codemirror/view";
import { json, jsonParseLinter } from "@codemirror/lang-json";
import { StreamLanguage, syntaxHighlighting } from "@codemirror/language";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { linter, lintGutter } from "@codemirror/lint";
import { oneDarkHighlightStyle } from "@codemirror/theme-one-dark";

function isDark(): boolean {
  return document.documentElement.classList.contains("dark");
}

/** 外观跟随应用配色：底色、文字、边框取应用的 CSS 变量；暗色下换一套高亮配色。 */
function appearance(dark: boolean) {
  return [
    EditorView.theme(
      {
        "&": {
          height: "100%",
          backgroundColor: "hsl(var(--background))",
          color: "hsl(var(--foreground))",
          fontSize: "12px",
        },
        "&.cm-focused": { outline: "none" },
        ".cm-scroller": { overflow: "auto", fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" },
        ".cm-content": { caretColor: "hsl(var(--foreground))" },
        ".cm-gutters": {
          backgroundColor: "hsl(var(--muted))",
          color: "hsl(var(--muted-foreground))",
          borderRight: "1px solid hsl(var(--border))",
        },
        ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: "hsl(var(--muted) / 0.6)" },
      },
      { dark },
    ),
    ...(dark ? [syntaxHighlighting(oneDarkHighlightStyle)] : []),
  ];
}

/**
 * CodeMirror 6 编辑器：行号、括号匹配、查找替换、撤销；JSON 另有行内语法检查，TOML 只做高亮。
 *
 * 受控用法：value 由外部持有，外部改了（重新读取、放弃改动、格式化）就整篇替换。
 */
export default function CodeEditor({
  value,
  onChange,
  language,
  ariaLabel,
  placeholder = "",
  testId,
}: {
  value: string;
  onChange: (value: string) => void;
  language: "json" | "toml";
  ariaLabel: string;
  placeholder?: string;
  testId?: string;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useEffect(() => {
    const theme = new Compartment();
    const view = new EditorView({
      parent: hostRef.current!,
      state: EditorState.create({
        doc: value,
        extensions: [
          basicSetup,
          language === "json" ? [json(), linter(jsonParseLinter()), lintGutter()] : StreamLanguage.define(toml),
          placeholderText(placeholder),
          EditorView.contentAttributes.of({ "aria-label": ariaLabel }),
          theme.of(appearance(isDark())),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onChangeRef.current(update.state.doc.toString());
          }),
        ],
      }),
    });
    viewRef.current = view;
    // 应用切换亮暗时换配色
    const observer = new MutationObserver(() => view.dispatch({ effects: theme.reconfigure(appearance(isDark())) }));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => {
      observer.disconnect();
      view.destroy();
      viewRef.current = null;
    };
    // 只在语言变化时重建；内容同步见下一个 effect
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [language]);

  useEffect(() => {
    const view = viewRef.current;
    if (view && view.state.doc.toString() !== value) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: value } });
    }
  }, [value]);

  return <div ref={hostRef} data-testid={testId} className="h-80 overflow-hidden rounded-md border" />;
}
