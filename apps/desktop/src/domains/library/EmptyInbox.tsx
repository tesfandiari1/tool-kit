import { Mono, Text } from "@ui";

/// A project with nothing in it yet.
///
/// Deliberately no import button. The drop target is the whole window and ⌘O
/// is the keyboard route, so a button here would be a third way to do the same
/// thing that then disappears the moment the first document lands, taking the
/// user's mental model of "where do I click" with it.
export function EmptyInbox({ projectTitle }: { projectTitle: string }) {
  return (
    <div className="library-empty">
      <Text size="sm" tone="faint">
        {projectTitle} is empty.
      </Text>
      <Text size="xs" tone="ghost">
        Drop files here to bring them in.
      </Text>
      {/* Ghost until the shortcut is wired: the slot is the promise, and a lit
          glyph would be a claim. */}
      <Mono size="xs" tone="ghost">
        ⌘O
      </Mono>
    </div>
  );
}
