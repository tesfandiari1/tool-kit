import { MarkdownViewer } from "./MarkdownViewer";

/// Dispatches an artifact to the right reader. Markdown is the only live
/// kind; audio playback and speaker tags wait on transcription APIs that
/// this conversion service does not expose.
export function FileViewer({
  text,
  empty = "Nothing to show",
}: {
  text: string | null;
  empty?: string;
}) {
  if (!text) return <div className="hist-empty">{empty}</div>;
  return <MarkdownViewer text={text} />;
}
