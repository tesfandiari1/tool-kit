import { useEffect, useRef } from "react";
import { EditorState, Compartment } from "@codemirror/state";
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

/// Syntax highlighting with no hue in it.
///
/// The first rule of this system is that colour is signal: amber is live,
/// green passed, red failed, cobalt the control you press. A conventional
/// syntax theme would spend four more colours on markdown and leave the user
/// unable to tell a heading from a running job at a glance.
///
/// So the structure is drawn with the ink ramp and weight instead. Headings
/// brighten and thicken, the marks that produce them (`##`, `**`, `>`) fall
/// back to the ghost tone, and code sits at the body tone in mono. It reads as
/// structure without spending a single signal colour.
const inkOnly = HighlightStyle.define([
  { tag: tags.heading, color: "var(--ink)", fontWeight: "600" },
  { tag: tags.heading1, color: "var(--ink)", fontWeight: "700" },
  { tag: tags.strong, color: "var(--ink)", fontWeight: "600" },
  { tag: tags.emphasis, color: "var(--ink-2)", fontStyle: "italic" },
  { tag: tags.link, color: "var(--ink-2)", textDecoration: "underline" },
  { tag: tags.url, color: "var(--ink-3)" },
  { tag: tags.monospace, color: "var(--ink-2)" },
  { tag: tags.quote, color: "var(--ink-2)", fontStyle: "italic" },
  { tag: tags.list, color: "var(--ink-2)" },
  /// Every syntactic mark the author typed but does not want to look at.
  { tag: tags.processingInstruction, color: "var(--ink-ghost)" },
  { tag: tags.meta, color: "var(--ink-ghost)" },
  { tag: tags.contentSeparator, color: "var(--ink-ghost)" },
]);

/// Chrome. Everything that is not the text itself, mapped onto tokens so a
/// theme swap moves the editor with the rest of the app.
const chrome = EditorView.theme({
  "&": {
    height: "100%",
    color: "var(--ink-2)",
    backgroundColor: "transparent",
    fontSize: "var(--t-sm)",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": {
    fontFamily: "var(--font-mono)",
    lineHeight: "1.62",
    overflow: "auto",
  },
  ".cm-content": {
    padding: "var(--s4) 0",
    caretColor: "var(--accent-hover)",
  },
  ".cm-line": { padding: "0 var(--s4)" },
  ".cm-gutters": {
    backgroundColor: "transparent",
    border: "none",
    borderRight: "1px solid var(--rule)",
    color: "var(--ink-ghost)",
    /// Line numbers tick, so they get the same tabular treatment as every
    /// other number in the app.
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
    color: "var(--ink-3)",
  },
  ".cm-activeLine": { backgroundColor: "var(--surface-well)" },
  /* `drawSelection` paints its own layer, and CodeMirror's base theme targets
     it as `.cm-selectionLayer .cm-selectionBackground`. Matching that
     specificity is the point: the single-class rule this replaces lost to the
     base theme, so a select-all came out in WebKit's default lavender. The
     native `::selection` is here too, for the unfocused editor and for the
     read surface beside it. */
  ".cm-selectionLayer .cm-selectionBackground, &.cm-focused .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection, ::selection":
    {
      backgroundColor: "var(--select)",
    },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent-hover)" },
});
/// Deliberately no `{ dark: true }`. That flag is baked in at module load, and
/// this app has two themes the user can be in — graphite or paper — so any
/// static answer is wrong half the time. Nothing here needs it: every colour
/// the editor draws is overridden above and resolves from the token layer, so
/// the editor follows the theme without being told which one it is in.

export interface SourceEditorProps {
  value: string;
  onChange?: (value: string) => void;
  readOnly?: boolean;
  /// Announced as the editor's name, since the text area has no visible label.
  label?: string;
  className?: string;
}

/// A plain-text editing surface with a line-number gutter.
///
/// CodeMirror rather than a textarea, for the one reason that matters here: a
/// gutter must stay aligned with soft-wrapped lines. A textarea plus a
/// hand-drawn column of numbers drifts the moment a line wraps, and a
/// conversion of a 31-page PDF wraps constantly.
///
/// The library knows nothing about the product. It takes a string and hands
/// one back.
export function SourceEditorImpl({ value, onChange, readOnly = false, label, className }: SourceEditorProps) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const editable = useRef(new Compartment());
  /// Held in a ref so the mount effect never depends on it. Depending on the
  /// prop would tear down and rebuild the editor on every parent render,
  /// losing the cursor, the selection and the whole undo history with it.
  const emit = useRef(onChange);
  /// Kept fresh in an effect rather than assigned during render: a ref write
  /// in the render body is a side effect, and React may discard that render.
  /// `useRef` already seeded it with the first `onChange`, so the mount effect
  /// below always has a live callback.
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
          editable.current.of(EditorView.editable.of(!readOnly)),
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
    // Mount once. `value` seeds the initial document; the effect below keeps
    // it in sync afterwards.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /// Pull in a change that came from outside — a different document opening,
  /// or a reload from disk. Guarded on inequality, because echoing the user's
  /// own keystroke back into the document resets their cursor to the start on
  /// every character typed.
  useEffect(() => {
    const v = view.current;
    if (!v || v.state.doc.toString() === value) return;
    v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: value } });
  }, [value]);

  useEffect(() => {
    view.current?.dispatch({
      effects: editable.current.reconfigure(EditorView.editable.of(!readOnly)),
    });
  }, [readOnly]);

  return <div ref={host} className={className ? `ui-source ${className}` : "ui-source"} role="group" aria-label={label} />;
}
