import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";

/// Reading surface for conversion output. Colour stays out of the markup —
/// headings and code inherit the instrument face, they don't get accent paint.
///
/// `remark-gfm` is not optional here. react-markdown parses CommonMark, which
/// has no tables, and a Datalab conversion of anything with a grid in it comes
/// back as a GFM table. Without this the app rendered them as rows of literal
/// pipe characters, while `App.css` carried table styles that could never
/// match a single element.
export function MarkdownViewer({ text }: { text: string }) {
  return (
    <div className="preview-body md">
      <Markdown remarkPlugins={[remarkGfm]}>{text}</Markdown>
    </div>
  );
}
