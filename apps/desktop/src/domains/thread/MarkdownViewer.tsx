import { memo, type ComponentProps } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { openExternal } from "@/platform/host";

/// Reading surface for conversion output. `remark-gfm` is not optional:
/// react-markdown parses CommonMark, which has no tables, so a converted grid
/// renders as rows of literal pipe characters without it.

/// Leaving is all a link here can do, and a relative path would navigate the
/// app off its own page. Anything else renders as text, because the opener
/// refuses it silently.
function MarkdownLink({ href, children }: ComponentProps<"a">) {
  if (href === undefined || !/^(?:https?|mailto|tel):/i.test(href)) {
    return <span title={href}>{children}</span>;
  }
  return (
    <a
      href={href}
      onClick={(e) => {
        e.preventDefault();
        void openExternal(href);
      }}
    >
      {children}
    </a>
  );
}

/// Memoized: react-markdown re-parses the whole document on every render, and
/// a 200-file run emits hundreds of them.
export const MarkdownViewer = memo(function MarkdownViewer({ text }: { text: string }) {
  return (
    <div className="preview-body md ui-selectable">
      <Markdown remarkPlugins={[remarkGfm]} components={{ a: MarkdownLink }}>
        {text}
      </Markdown>
    </div>
  );
});
