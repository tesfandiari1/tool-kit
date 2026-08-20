import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Button, Display, Path, Row, Segmented, Stack, Text } from "@ui";
import { commands } from "@/app/commands";
import { tildePath } from "@/app/format";
import type { OnboardingConversionMode, WorkspaceInfo } from "@/app/types";
import { pickDirectory } from "@/platform/host";
import { conversionConsequence } from "./conversionMode";

/// Three beats, one question each. The user is not configuring an app here,
/// they are being told what it is and agreeing to two defaults, so every beat
/// is a sentence and a single control.
type Step = "hello" | "workspace" | "conversion";

/// A folder the CTA could act on, and what acting on it would mean.
interface Candidate {
  path: string;
  /// A workspace is already there, so setup adopts it. It is the difference
  /// between the button saying Create and saying Open.
  existing: boolean;
}

export function OnboardingGate({
  onDone,
  onToast,
}: {
  onDone: (workspace: WorkspaceInfo, mode: OnboardingConversionMode) => void;
  onToast: (message: string) => void;
}) {
  const [step, setStep] = useState<Step>("hello");
  /// The folder the CTA is about to act on. Null only while the host is still
  /// answering, which is why the button waits rather than guessing a path.
  const [candidate, setCandidate] = useState<Candidate | null>(null);
  const [workspace, setWorkspace] = useState<WorkspaceInfo | null>(null);
  const [mode, setMode] = useState<OnboardingConversionMode>("local");
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
      const info = await commands.setupWorkspace(candidate.path);
      setWorkspace(info);
      setStep("conversion");
    } catch (e) {
      onToast(String(e));
    } finally {
      setBusy(false);
    }
  }, [candidate, onToast]);

  const advance = useCallback(() => {
    if (busy) return;
    switch (step) {
      case "hello":
        setStep("workspace");
        return;
      case "workspace":
        void setup();
        return;
      case "conversion":
        // The workspace exists by now: beat 3 is only reachable through a
        // setup that resolved.
        if (workspace) onDone(workspace, mode);
        return;
      default: {
        const _exhaustive: never = step;
        return _exhaustive;
      }
    }
  }, [busy, mode, onDone, setup, step, workspace]);

  // Return is the whole keyboard path through the gate.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Enter" || e.defaultPrevented) return;
      // A focused button answers Return itself, so advancing here as well
      // would skip a beat or open a picker and move on at the same time. The
      // segmented is the exception: re-selecting the mode you are already on
      // does nothing, so Return there should carry on rather than stall.
      const el = document.activeElement;
      if (el instanceof HTMLElement && el.matches("button:not([role='radio'])")) return;
      e.preventDefault();
      advance();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [advance]);

  let beat: ReactNode;
  let cta: string;
  let ready = true;
  switch (step) {
    case "hello":
      beat = (
        <>
          <Display as="h1" size="3xl">
            Tool-Kit
          </Display>
          <Text tone="muted">
            A local markdown workspace. Bring documents in, work on them, and keep every
            file on your own disk.
          </Text>
        </>
      );
      cta = "Continue";
      break;
    case "workspace":
      beat = (
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
                <Button variant="link" onClick={() => void choose()}>
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
      cta = candidate?.existing ? "Open workspace" : "Create workspace";
      ready = candidate !== null;
      break;
    case "conversion":
      beat = (
        <>
          <Display as="h1" size="2xl">
            Where conversion runs
          </Display>
          <Segmented
            label="Where conversion runs"
            value={mode}
            onChange={setMode}
            options={[
              { value: "local", label: "This Mac" },
              { value: "cloud", label: "Cloud" },
            ]}
          />
          <Text size="sm" tone="muted">
            {conversionConsequence(mode)}
          </Text>
          <Text size="xs" tone="ghost">
            You can change this later in Settings.
          </Text>
        </>
      );
      cta = "Open workspace";
      break;
    default: {
      const _exhaustive: never = step;
      beat = _exhaustive;
      cta = "";
    }
  }

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
