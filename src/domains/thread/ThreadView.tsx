import { CopyIcon, FolderOpenIcon, XIcon } from "@phosphor-icons/react";
import { Button, Label, Row, Spacer, Text } from "@ui";
import { FileViewer } from "./FileViewer";

/// One file from a run, opened as a reading surface. This is a thread in
/// the Yaak-inspector sense (list item → body), not a chat transcript.
export function ThreadView({
  title,
  subtitle,
  text,
  onCopy,
  onReveal,
  onClose,
}: {
  title: string;
  subtitle: string | null;
  text: string | null;
  onCopy: () => void;
  onReveal: (() => void) | null;
  onClose: () => void;
}) {
  return (
    <main className="flow">
      <Row gap={3}>
        <Label tone="strong" className="ui-truncate" title={title}>
          {title}
        </Label>
        <Spacer />
        {text && (
          <Button
            variant="ghost"
            size="sm"
            iconOnly
            icon={<CopyIcon />}
            title="Copy"
            aria-label="Copy"
            onClick={onCopy}
          />
        )}
        {onReveal && (
          <Button
            variant="ghost"
            size="sm"
            iconOnly
            icon={<FolderOpenIcon />}
            title="Show in Finder"
            aria-label="Show in Finder"
            onClick={onReveal}
          />
        )}
        <Button variant="ghost" iconOnly icon={<XIcon />} onClick={onClose} aria-label="Close" />
      </Row>
      {subtitle && (
        <Text size="xs" tone="faint" truncate title={subtitle}>
          {subtitle}
        </Text>
      )}
      <FileViewer text={text} empty="No extracted text for this file" />
    </main>
  );
}
