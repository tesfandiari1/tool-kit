import Markdown from "react-markdown";

/// Reading surface for conversion output. Colour stays out of the markup —
/// headings and code inherit the instrument face, they don't get accent paint.
export function MarkdownViewer({ text }: { text: string }) {
  return (
    <div className="preview-body md">
      <Markdown>{text}</Markdown>
    </div>
  );
}
