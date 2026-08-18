import { useEffect, useRef, useState } from "react";
import { commands } from "@/app/commands";
import { confirm, currentWindow, onDragDrop } from "@/platform/host";

/// The window itself: how files get in, how the chrome reacts to focus, and
/// what closing means. Three separate host subscriptions, each with its own
/// unsubscribe, grouped because they are all "the OS talking to us" and none
/// of them is about jobs.

/// Files arrive by drop anywhere in the window. Returns whether a drag is
/// currently over it, which the input well renders as an affordance.
export function useDragDrop(onDrop: (paths: string[]) => void) {
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    const un = onDragDrop((p) => {
      if (p.type === "over" || p.type === "enter") setDragging(true);
      else if (p.type === "drop") {
        setDragging(false);
        onDrop(p.paths);
      } else setDragging(false);
    });
    return () => void un.then((f) => f());
  }, [onDrop]);

  return dragging;
}

/// macOS dims an inactive window's accents; mirror it on `<body>` so the
/// stylesheet can follow with `body.inactive`.
export function useWindowFocusClass() {
  useEffect(() => {
    const win = currentWindow();
    void win.isFocused().then((f) => document.body.classList.toggle("inactive", !f));
    const un = win.onFocusChanged(({ payload }) =>
      document.body.classList.toggle("inactive", !payload),
    );
    return () => void un.then((f) => f());
  }, []);
}

/// This is a menu-bar app: closing the window hides it rather than quitting,
/// so the tray and ⌥⌘V keep working. Quit via the tray menu or ⌘Q. Closing
/// mid-run would abandon files already paid for upstream, so that asks first.
///
/// The handler is registered once and reads `running` / `activeCount` /
/// `dirtyCount` through refs. Re-registering on every job event would be a
/// subscription churn on a 200-file run, and the closure would still be one
/// render stale at the moment it matters.
export function useCloseConfirm(running: boolean, activeCount: number, dirtyCount: number) {
  const runningRef = useRef(running);
  const activeRef = useRef(activeCount);
  const dirtyRef = useRef(dirtyCount);

  useEffect(() => {
    runningRef.current = running;
    activeRef.current = activeCount;
    dirtyRef.current = dirtyCount;
  }, [running, activeCount, dirtyCount]);

  useEffect(() => {
    const win = currentWindow();
    const un = win.onCloseRequested(async (e) => {
      e.preventDefault();
      // Hiding keeps the webview alive, so an unsaved edit survives it. Quitting
      // does not, and the quit below is one click away — so an edit that is
      // still only in memory has to be said out loud before either.
      //
      // Reached when the autosave has not landed yet, or when the host refused
      // the write because the file changed underneath us. That second case is
      // the one that matters: it will not fix itself by waiting.
      const dirty = dirtyRef.current;
      if (dirty > 0) {
        const keep = await confirm(
          `${String(dirty)} document${dirty > 1 ? "s have" : " has"} changes that are not on disk. ` +
            "Leave the window open to finish saving, or hide it and keep them in memory?",
          {
            title: dirty > 1 ? "Unsaved documents" : "Unsaved document",
            kind: "warning",
            okLabel: "Keep open",
            cancelLabel: "Hide anyway",
          },
        );
        if (keep) return;
      }
      if (!runningRef.current) {
        await win.hide();
        return;
      }
      const active = activeRef.current;
      const keepGoing = await confirm(
        `${active} file${active > 1 ? "s are" : " is"} still processing. ` +
          "Keep the run going in the menu bar, or stop it and quit?",
        {
          title: "Tool-Kit is still working",
          kind: "warning",
          okLabel: "Keep working",
          cancelLabel: "Stop and quit",
        },
      );
      if (keepGoing) await win.hide();
      else {
        await commands.stopRun().catch(() => undefined);
        await commands.quitApp().catch(() => undefined);
      }
    });
    return () => void un.then((f) => f());
  }, []);
}
