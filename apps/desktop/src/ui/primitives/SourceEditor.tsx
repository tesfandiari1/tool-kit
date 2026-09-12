import { Suspense, lazy } from "react";
import type { SourceEditorProps } from "./SourceEditorImpl";
import "./SourceEditor.css";

export type { SourceEditorProps } from "./SourceEditorImpl";

/// CodeMirror is ~350KB and the common path never opens it, so it is fetched
/// on the first press of EDIT. The chunk comes off local disk.
const Impl = lazy(() =>
  import("./SourceEditorImpl").then((m) => ({ default: m.SourceEditorImpl })),
);

/// A plain-text editing surface with a line-number gutter. It takes a string
/// and hands one back.
export function SourceEditor(props: SourceEditorProps) {
  return (
    /* The fallback fills the editor's box, so nothing shifts on arrival. */
    <Suspense fallback={<div className="ui-source" aria-busy="true" />}>
      <Impl {...props} />
    </Suspense>
  );
}
