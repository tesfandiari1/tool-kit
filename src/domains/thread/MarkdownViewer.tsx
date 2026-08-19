import { memo, type ComponentProps } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { openExternal } from "@/platform/host";

/// Reading surface for conversion output. Headings and code use the prose stack
/// in `.md`; accent paint stays out of the markup.
///
/// `remark-gfm` is not optional here. react-markdown parses CommonMark, which
/// has no tables, and a Datalab conversion of anything with a grid in it comes
/// back as a GFM table. Without this the app rendered them as rows of literal
/// pipe characters, while `App.css` carried table styles that could never
/// match a single element.
function MarkdownLink({ href, children }: ComponentProps<"a">) {
  return (
    <a
      href={href}
      onClick={(e) => {
        e.preventDefault();
        if (href) void openExternal(href);
      }}
    >
      {children}
    </a>
  );
}

/// Memoized because react-markdown does not memoize itself: every render
/// re-parses the whole document. A run emits a `job-updated` per queued job,
/// four more per file, and a 1 Hz elapsed-time tick, so an open document was
/// being re-parsed roughly a thousand times over a 200-file run while its own
/// text never changed.
export const MarkdownViewer = memo(function MarkdownViewer({ text }: { text: string }) {
  return (
    <div className="preview-body md">
      <Markdown remarkPlugins={[remarkGfm]} components={{ a: MarkdownLink }}>
        {text}
      </Markdown>
    </div>
  );
});
