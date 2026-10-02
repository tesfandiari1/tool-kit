import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  drawSelection,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { markdown } from "@codemirror/lang-markdown";
import { tags } from "@lezer/highlight";

/// Highlighting with no hue: colour is signal here, so structure is drawn with
/// the ink ramp and weight (UI.md rule 1).
const inkOnly = HighlightStyle.define([
  { tag: tags.heading, color: "var(--text-body)", fontWeight: "600" },
  { tag: tags.heading1, color: "var(--text-body)", fontWeight: "700" },
  { tag: tags.strong, color: "var(--text-body)", fontWeight: "600" },
  { tag: tags.emphasis, color: "var(--text-soft)", fontStyle: "italic" },
  { tag: tags.link, color: "var(--text-soft)", textDecoration: "underline" },
  { tag: tags.url, color: "var(--text-mute)" },
  { tag: tags.monospace, color: "var(--text-soft)" },
  { tag: tags.quote, color: "var(--text-soft)", fontStyle: "italic" },
  { tag: tags.list, color: "var(--text-soft)" },
  /// Marks the author typed and does not want to look at.
  { tag: tags.processingInstruction, color: "var(--text-ghost)" },
  { tag: tags.meta, color: "var(--text-ghost)" },
  { tag: tags.contentSeparator, color: "var(--text-ghost)" },
]);

/// Chrome, on tokens, so a theme swap moves the editor with the app.
const chrome = EditorView.theme({
  "&": {
    height: "100%",
    color: "var(--text-soft)",
    backgroundColor: "transparent",
    fontSize: "var(--t-sm)",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": {
    fontFamily: "var(--font-code)",
    lineHeight: "1.62",
    overflow: "auto",
  },
  ".cm-content": {
    padding: "var(--s4) 0",
    caretColor: "var(--focus-ring)",
  },
  ".cm-line": { padding: "0 var(--s4)" },
  ".cm-gutters": {
    backgroundColor: "transparent",
    border: "none",
    borderRight: "1px solid var(--rule)",
    color: "var(--text-ghost)",
    /// Line numbers tick, so they set tabular (UI.md rule 2).
    fontVariantNumeric: "tabular-nums",
    paddingRight: "var(--s1)",
    minWidth: "44px",
  },
  ".cm-lineNumbers .cm-gutterElement": {
    padding: "0 var(--s2) 0 var(--s3)",
    textAlign: "right",
  },
  ".cm-activeLineGutter": {
    backgroundColor: "transparent",
    color: "var(--text-mute)",
  },
  ".cm-activeLine": { backgroundColor: "var(--surface-lift)" },
  /* Match the base theme's `.cm-selectionLayer .cm-selectionBackground`
     specificity, or a select-all comes out in WebKit's lavender. */
  ".cm-selectionLayer .cm-selectionBackground, &.cm-focused .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection, ::selection":
    {
      backgroundColor: "var(--selection)",
    },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--focus-ring)" },
});
/// No `{ dark: true }`: it is baked in at module load, and the app has two
/// themes. Every colour resolves from the token layer instead.

export interface SourceEditorProps {
  value: string;
  onChange?: (value: string) => void;
  label?: string;
}

/// CodeMirror rather than a textarea for one reason: a gutter has to stay
/// aligned with soft-wrapped lines.
export function SourceEditorImpl({ value, onChange, label }: SourceEditorProps) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  /// In a ref, or the mount effect rebuilds the editor on every parent render
  /// and loses the cursor and the undo history.
  const emit = useRef(onChange);
  /// Written in an effect: a ref write during render is a side effect.
  useEffect(() => {
    emit.current = onChange;
  }, [onChange]);

  useEffect(() => {
    if (!host.current) return;
    const v = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc: value,
        extensions: [
          lineNumbers(),
          highlightActiveLine(),
          highlightActiveLineGutter(),
          drawSelection(),
          history(),
          keymap.of([...defaultKeymap, ...historyKeymap]),
          markdown(),
          syntaxHighlighting(inkOnly),
          EditorView.lineWrapping,
          chrome,
          EditorView.updateListener.of((u) => {
            if (u.docChanged) emit.current?.(u.state.doc.toString());
          }),
        ],
      }),
    });
    view.current = v;
    return () => {
      v.destroy();
      view.current = null;
    };
    // Mount once. `value` seeds the document and the effect below syncs it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /// A change from outside. Guarded on inequality: echoing the user's own
  /// keystroke back resets the cursor on every character. The document holds
  /// `\n` only, so a CRLF file compares as normalized. Raw, the guard misses on
  /// mount and the replace reports an edit, which autosaves the file as LF.
  useEffect(() => {
    const v = view.current;
    if (!v || v.state.doc.toString() === value.replace(/\r\n?/g, "\n")) return;
    v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: value } });
  }, [value]);

  return <div ref={host} className="ui-source" role="group" aria-label={label} />;
}
