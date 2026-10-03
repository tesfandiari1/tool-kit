import { useCallback, useEffect, useState } from "react";
import { Button, Display, Path, Row, Stack, Text } from "@ui";
import { commands } from "@/app/commands";
import { tildePath } from "@/app/format";
import type { WorkspaceInfo } from "@/app/types";
import { pickDirectory } from "@/platform/host";

/// Two beats, each a sentence and one control.
type Step = "hello" | "workspace";

/// A folder the button could act on, and what that would mean.
interface Candidate {
  path: string;
  /// Setup will adopt it: the button says Open rather than Create.
  existing: boolean;
}

export function OnboardingGate({
  onDone,
  onToast,
}: {
  onDone: (workspace: WorkspaceInfo) => void;
  onToast: (message: string) => void;
}) {
  const [step, setStep] = useState<Step>("hello");
  /// Null while the host is answering, so the button waits.
  const [candidate, setCandidate] = useState<Candidate | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const load = { cancelled: false };
    void (async () => {
      try {
        const suggested = await commands.suggestedWorkspacePath();
        const existing = await commands.inspectWorkspacePath(suggested);
        if (!load.cancelled) setCandidate({ path: suggested, existing });
      } catch (e) {
        if (!load.cancelled) onToast(String(e));
      }
    })();
    return () => {
      load.cancelled = true;
    };
  }, [onToast]);

  const choose = useCallback(async () => {
    const dir = await pickDirectory();
    // Cancelling a picker is not an error and not a decision. Say nothing.
    if (dir === null) return;
    try {
      setCandidate({ path: dir, existing: await commands.inspectWorkspacePath(dir) });
    } catch (e) {
      onToast(String(e));
    }
  }, [onToast]);

  const setup = useCallback(async () => {
    if (!candidate) return;
    setBusy(true);
    try {
      onDone(await commands.setupWorkspace(candidate.path));
    } catch (e) {
      onToast(String(e));
    } finally {
      setBusy(false);
    }
  }, [candidate, onDone, onToast]);

  const hello = step === "hello";

  const advance = useCallback(() => {
    if (busy) return;
    if (hello) setStep("workspace");
    else void setup();
  }, [busy, setup, hello]);

  // Return is the whole keyboard path through the gate.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Enter" || e.defaultPrevented) return;
      // A focused button answers Return itself, so advancing here skips a
      // beat.
      const el = document.activeElement;
      if (el instanceof HTMLElement && el.matches("button")) return;
      e.preventDefault();
      advance();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [advance]);

  const beat = hello ? (
    <>
      <Display as="h1" size="3xl">
        Tool-Kit
      </Display>
      <Text tone="muted">
        A local markdown workspace. Bring documents in, work on them, and keep every
        file on your own disk.
      </Text>
    </>
  ) : (
    <>
      <Display as="h1" size="2xl">
        One folder holds everything
      </Display>
      <Text tone="muted">
        Projects are folders inside it and documents are plain markdown files. You can
        open them in any other app, and back them up like the rest of your disk.
      </Text>
      {candidate ? (
        <Stack gap={2}>
          <Path path={tildePath(candidate.path)} />
          <Row gap={2}>
            <Button variant="ghost" onClick={() => void choose()}>
              Choose different folder
            </Button>
          </Row>
        </Stack>
      ) : (
        <Text size="sm" tone="ghost">
          Finding a place for it…
        </Text>
      )}
    </>
  );
  const cta = hello ? "Continue" : candidate?.existing ? "Open workspace" : "Create workspace";
  const ready = hello || candidate !== null;

  return (
    <main className="onboarding-gate">
      {/* Keyed on the step so each beat mounts and fades in. The foot is
          outside it on purpose: the CTA is the one thing that must not move
          or flicker between beats. */}
      <div className="onboarding-beat" key={step}>
        {beat}
      </div>
      <div className="onboarding-foot">
        <Button
          variant="primary"
          size="lg"
          block
          busy={busy}
          disabled={busy || !ready}
          onClick={advance}
        >
          {cta}
        </Button>
      </div>
    </main>
  );
}
