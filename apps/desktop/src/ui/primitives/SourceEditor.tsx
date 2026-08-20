import { Suspense, lazy } from "react";
import type { SourceEditorProps } from "./SourceEditorImpl";
import "./SourceEditor.css";

export type { SourceEditorProps } from "./SourceEditorImpl";

/// CodeMirror is ~350KB of the bundle, and the app's common path never opens
/// it: you drop twenty files, press Run and walk away. Read mode is the
/// default when a document does open, so the editor is fetched on the first
/// press of EDIT and not before. On a desktop app the chunk comes off local
/// disk, so the wait is a frame, not a spinner.
const Impl = lazy(() =>
  import("./SourceEditorImpl").then((m) => ({ default: m.SourceEditorImpl })),
);

/// A plain-text editing surface with a line-number gutter.
///
/// CodeMirror rather than a textarea, for the one reason that matters here: a
/// gutter must stay aligned with soft-wrapped lines. A textarea plus a
/// hand-drawn column of numbers drifts the moment a line wraps, and a
/// conversion of a 31-page PDF wraps constantly.
///
/// The library knows nothing about the product. It takes a string and hands
/// one back.
export function SourceEditor(props: SourceEditorProps) {
  return (
    /* The fallback fills the same box the editor will, so arriving text does
       not shift the pane under the reader's eyes. */
    <Suspense fallback={<div className="ui-source" aria-busy="true" />}>
      <Impl {...props} />
    </Suspense>
  );
}
